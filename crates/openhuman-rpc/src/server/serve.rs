//! Bind the core's HTTP listener and serve until shutdown.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::core_host::core::runtime::CoreRuntime;

/// Metadata sent back to the Tauri host once the embedded core has selected
/// and bound its listen port.
#[derive(Debug, Clone)]
pub struct EmbeddedReadySignal {
    pub port: u16,
    pub fallback_from: Option<u16>,
}

async fn await_http_server(
    server: &mut tokio::task::JoinHandle<std::io::Result<()>>,
    shutdown_token: Option<&CancellationToken>,
) -> anyhow::Result<()> {
    let join_result = if let Some(shutdown_token) = shutdown_token {
        tokio::select! {
            result = &mut *server => result,
            _ = shutdown_token.cancelled() => {
                match tokio::time::timeout(std::time::Duration::from_secs(10), &mut *server).await {
                    Ok(result) => result,
                    Err(_) => {
                        // The server has had ten seconds to drain active
                        // requests after the host's shutdown signal.
                        server.abort();
                        let _ = server.await;
                        return Ok(());
                    }
                }
            }
        }
    } else {
        server.await
    };

    join_result
        .map_err(anyhow::Error::from)?
        .map_err(anyhow::Error::from)
}

/// Spawn `runtime`'s selected background services and, when
/// `ServiceSet::rpc_http` is set, bind the HTTP listener and serve until
/// shutdown.
///
/// When `rpc_http` is not selected this returns once the services are spawned
/// (a harness-only embedder has no transport to run).
///
/// With `shutdown_token` the server stops when it is cancelled (the embedded
/// desktop core); without one it stops on the process shutdown signal.
pub async fn serve(
    runtime: Arc<CoreRuntime>,
    ready_tx: Option<tokio::sync::oneshot::Sender<EmbeddedReadySignal>>,
    shutdown_token: Option<CancellationToken>,
) -> anyhow::Result<()> {
    if !runtime.services().rpc_http {
        // No transport: just spawn the selected background services and
        // return. The caller owns the process lifetime.
        runtime.start_services().await;
        return Ok(());
    }

    // --- Host / port resolution ---
    let (resolved_port, port_source) = match runtime.port() {
        Some(p) => (p, "builder port"),
        None => (
            super::shims::core_port(),
            if std::env::var("OPENHUMAN_CORE_PORT").is_ok() {
                "env OPENHUMAN_CORE_PORT"
            } else {
                "default"
            },
        ),
    };
    let (resolved_host, host_source) = match runtime.host() {
        Some(h) => (h.to_string(), "builder host"),
        None => (
            super::shims::core_host(),
            if std::env::var("OPENHUMAN_CORE_HOST")
                .ok()
                .filter(|s| !s.is_empty())
                .is_some()
            {
                "env OPENHUMAN_CORE_HOST"
            } else {
                "default"
            },
        ),
    };

    log::debug!(
            "[core] Bind resolution: host={resolved_host} (from {host_source}), port={resolved_port} (from {port_source})"
        );

    // Safety check: refuse to bind on a non-loopback address without an
    // explicit operator-supplied RPC token. Without this, the entire RPC
    // surface (tool execution, file access, credentials) is unauthenticated
    // and reachable from the network. See issue #1919. The self-generated
    // {workspace}/core.token does NOT count — remote clients cannot read it,
    // so treating it as "explicit" would be fail-open.
    if crate::core_host::security::pairing::is_public_bind(&resolved_host)
        && !runtime.has_operator_token()
    {
        log::error!(
            "[core] SECURITY: refusing to bind on public address {resolved_host} without an \
                 explicit operator-supplied RPC token. Set {} in your environment (or hand the \
                 bearer in-memory via the embedded core handle) to secure the RPC endpoint.",
            crate::core_host::core::auth::CORE_TOKEN_ENV_VAR
        );
        eprintln!(
            "\n\x1b[1;31m[SECURITY]\x1b[0m Refusing to bind on {resolved_host} without {}.\n\
                 The auto-generated {{workspace}}/core.token does NOT secure a public bind —\n\
                 remote clients cannot read it. Set {} in your environment to secure the\n\
                 RPC endpoint, or bind on a loopback address.\n",
            crate::core_host::core::auth::CORE_TOKEN_ENV_VAR,
            crate::core_host::core::auth::CORE_TOKEN_ENV_VAR
        );
        anyhow::bail!(
            "refusing to bind on non-loopback address {resolved_host} without an explicit \
                 operator-supplied RPC token ({})",
            crate::core_host::core::auth::CORE_TOKEN_ENV_VAR
        );
    }

    let preferred_port = resolved_port;
    let host = resolved_host;
    // The desktop shell hands in `ready_tx` and owns the stale-listener
    // takeover (#1130) for its own leftover core. A headless `serve` has
    // nothing to take over: another live core on the port (the desktop
    // app's, another checkout's) is a neighbour, so move to a free port.
    let occupied_by_core = if ready_tx.is_some() {
        crate::core_host::platform::connectivity::rpc::OccupiedByCore::Takeover
    } else {
        crate::core_host::platform::connectivity::rpc::OccupiedByCore::Fallback
    };
    let pick = crate::core_host::platform::connectivity::rpc::pick_listen_port_for_host_with(
        host.as_str(),
        preferred_port,
        occupied_by_core,
    )
    .await
    .map_err(|err| {
        log::error!("[core] Failed to bind to {host}:{preferred_port}: {err}");
        anyhow::Error::new(err)
    })?;
    let listen_port = pick.port;
    let bind_addr = format!("{host}:{listen_port}");
    let listener = pick.listener;
    if let Ok(local_addr) = listener.local_addr() {
        runtime.listener_bound(local_addr);
    }

    // Synchronize OPENHUMAN_CORE_RPC_URL with the actual bound port so
    // connectivity::rpc::resolve_listen_port() reports the live listener
    // instead of the originally-requested port when fallback engaged.
    //
    // SAFETY: set_var is process-global; this runs once during bind. Flagged
    // in the pluggable-core drift ledger as single-runtime-per-process.
    unsafe {
        std::env::set_var("OPENHUMAN_CORE_RPC_URL", format!("http://{bind_addr}/rpc"));
    }

    let preflight_gate = runtime
        .requires_secret_preflight()
        .then(super::auth::PreflightGate::new_active);
    let ctx = Arc::clone(runtime.context());
    let router = match &preflight_gate {
        Some(gate) => super::http::build_core_http_router_with_preflight(
            runtime.normal_services().socketio,
            gate.clone(),
        ),
        None => super::http::build_core_http_router(runtime.services().socketio),
    };
    // A SaaS core scopes each request to the user the gateway names (or the
    // operator plane); a single-user core runs everything under its one
    // context.
    let app = if crate::core_host::core::runtime::is_saas() {
        router.layer(axum::middleware::from_fn(
            move |req: axum::extract::Request, next: axum::middleware::Next| {
                super::saas_gateway::saas_gateway(Arc::clone(&ctx), req, next)
            },
        ))
    } else {
        router.layer(axum::middleware::from_fn(
            move |req: axum::extract::Request, next: axum::middleware::Next| {
                let ctx = Arc::clone(&ctx);
                async move { super::saas_gateway::dispatch_in(ctx, req, next).await }
            },
        ))
    };

    // Normal boots finish migrations before readiness. Secret preflight must
    // instead publish the restricted listener so the operator can answer the
    // HumanOnce approval required to decrypt config; background writers stay
    // deferred until that phase succeeds.
    if preflight_gate.is_none() {
        runtime.start_services().await;
    }

    log::info!(
        "[core] OpenHuman core is ready — listening on http://{bind_addr} (version {})",
        crate::core_host::core::invoke::default_state().core_version
    );
    log::info!("[rpc:http] JSON-RPC — POST http://{bind_addr}/rpc (JSON-RPC 2.0)");
    if runtime.normal_services().socketio {
        log::info!("[rpc:socketio] Socket.IO — ws://{bind_addr}/socket.io/ (same HTTP server)");
    } else {
        log::info!("[rpc:socketio] disabled (--jsonrpc-only)");
    }

    if let Some(tx) = ready_tx {
        let _ = tx.send(EmbeddedReadySignal {
            port: listen_port,
            fallback_from: pick.fallback_from,
        });
    }

    // The serve result is held, not propagated, until the exit work below
    // has run. A `?` here on a server error would skip the memory teardown
    // on exactly the exits where a wedged store is likeliest, and the
    // callers only forward the error — nobody else runs the cleanup.
    let served = if let Some(gate) = preflight_gate {
        let server_shutdown = shutdown_token.clone();
        let shutdown = async move {
            if let Some(shutdown_token) = shutdown_token {
                shutdown_token.cancelled().await;
            } else {
                crate::core_host::core::shutdown::signal().await;
            }
        };
        // Keep the listener independent from startup work. In particular,
        // `start_services()` can take time after preflight; awaiting it in the
        // select branch used to stop polling Axum and leave even `/health`
        // hanging while the socket was already bound.
        let mut server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(shutdown)
                .await
        });
        let preflight_runtime = Arc::clone(&runtime);
        let mut preflight =
            tokio::spawn(async move { preflight_runtime.complete_secret_preflight().await });
        // Enter the shutdown select before the preflight task can run. A
        // preflight may do synchronous work before its first await, and must
        // not monopolize a runtime worker before cancellation is observed.
        tokio::select! {
            result = await_http_server(&mut server, server_shutdown.as_ref()) => {
                preflight.abort();
                result
            },
            result = &mut preflight => match result {
                Err(error) => {
                    server.abort();
                    Err(anyhow::Error::from(error))
                }
                Ok(Err(error)) => {
                    server.abort();
                    Err(error)
                }
                Ok(Ok(())) => {
                    let startup_runtime = Arc::clone(&runtime);
                    let mut service_start =
                        tokio::spawn(async move { startup_runtime.start_services().await });
                    tokio::select! {
                        result = await_http_server(&mut server, server_shutdown.as_ref()) => {
                            service_start.abort();
                            result
                        },
                        result = &mut service_start => {
                            match result {
                                Err(error) => Err(anyhow::Error::from(error)),
                                Ok(()) => {
                                    // Start ordinary jobs only after credentials and
                                    // config have been refreshed, then expose the full
                                    // router. A concurrent shutdown must still stop
                                    // the listener while these services are starting.
                                    gate.open();
                            await_http_server(&mut server, server_shutdown.as_ref()).await
                                }
                            }
                        }
                    }
                }
            }
        }
    } else if let Some(shutdown_token) = shutdown_token {
        log::info!("[core] embedded server waiting on cancellation token for graceful shutdown");
        let server_shutdown = shutdown_token.clone();
        let mut server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    shutdown_token.cancelled().await;
                })
                .await
        });
        eprintln!("serve-debug: non-preflight server started");
        await_http_server(&mut server, Some(&server_shutdown)).await
    } else {
        axum::serve(listener, app)
            .with_graceful_shutdown(crate::core_host::core::shutdown::signal())
            .await
            .map_err(anyhow::Error::from)
    };
    if let Err(error) = &served {
        log::warn!(
            "[core] embedded server ended with an error; running exit cleanup before \
                 reporting it: {error}"
        );
    }

    if tokio::time::timeout(std::time::Duration::from_secs(10), runtime.exit_cleanup())
        .await
        .is_err()
    {
        log::warn!("[core] runtime exit cleanup exceeded its 10-second shutdown budget");
    }
    // Close the per-workspace background-completion logs (they replay from disk
    // on the next boot) so a data reset can delete the workspace directory.
    if tokio::time::timeout(
        std::time::Duration::from_secs(10),
        crate::core_host::agent::orchestration::release_background_completion_stores(),
    )
    .await
    .is_err()
    {
        log::warn!("[core] background completion stores did not close within 10 seconds");
    }

    served?;
    Ok(())
}
