//! Login-gated background services (local AI) started on session store and
//! stopped on session clear, plus the embedder-host detection
//! that gates global (operator-wide) activation side effects.

use crate::config::Config;

/// Whether this dispatch is running under an embedder-hosted core (the library
/// `Harness`) rather than the desktop shell or CLI.
///
/// An embedder supplies its own scoped [`Config`] via `CoreBuilder::config`,
/// so its `auth_store_session` must keep activation and credential state under
/// that config's path and never touch the operator's global
/// `~/.openhuman/active_user.toml` / `users/` tree.
pub(crate) fn is_embedder_host() -> bool {
    crate::core::runtime::context::CoreContext::with_current_embedder_config(|_| ()).is_some()
}

/// Populate the process cache away from the next chat turn after startup or a
/// credential change. Connection changes have their own invalidation paths.
pub(super) fn spawn_integrations_cache_warm(config: &Config) {
    let integration_config = config.clone();
    tokio::spawn(async move {
        let started = std::time::Instant::now();
        match crate::integrations::composio::fetch_connected_integrations_status(
            &integration_config,
        )
        .await
        {
            crate::integrations::composio::FetchConnectedIntegrationsStatus::Authoritative(
                entries,
            ) => log::info!(
                "[services] integrations cache warmed entries={} elapsed_ms={}",
                entries.len(),
                started.elapsed().as_millis()
            ),
            crate::integrations::composio::FetchConnectedIntegrationsStatus::Unavailable => {
                log::debug!("[services] integrations cache warm unavailable; first turn may retry")
            }
        }
    });
}

/// Start login-gated background services. Called both
/// from the initial boot path (when an existing
/// session is detected) and from `set_credential()` when a credential is installed.
pub async fn start_credential_gated_services(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
) {
    // Optional TinyFlows resources warm in the background and must never hold
    // credential activation or ordinary chat startup on module availability.
    crate::skills::schedule_flow_authoring_prewarm(ctx, config, None);
    // These login-gated services are mutually independent — the ONLY ordering
    // constraint is voice-server → standalone-dictation-listener (they contend
    // for the single rdev global listener on macOS). Previously each was
    // `.await`ed in series, so their cold-start costs SUMMED: the local-AI
    // bootstrap (Ollama/embeddings) + the Windows WASAPI microphone init
    // (a synchronous readiness handshake in `always_on::spawn_capture_thread`)
    // stacked into the ~10s stall users hit before hotkeys/commands were usable
    // — worst on Windows (#3490). Worse,
    // the hotkey/command registration (steps 2–3) sat
    // *after* the local-AI bootstrap in the series, so commands could not
    // register until Ollama finished warming.
    //
    // Run them concurrently on independent tasks instead: readiness is bounded
    // by the slowest single service rather than their sum, and command
    // registration no longer waits behind local-AI warm-up. Each task logs its
    // own elapsed time so a future regression can be attributed to one stage; a
    // panic in one service is logged on join and never aborts the others.

    // Unit tests must not launch the real login-gated background services: they
    // include detached listeners that outlive the test that spawned them and interleave with the
    // shared process state (HOME / active_user.toml) of the parallel `cargo
    // test` run. Once startup became concurrent (#3490) that interleaving made
    // the session-isolation tests order-dependent. `cfg!(test)` is compiled out
    // of every production/release build, so this gate never affects shipped
    // behavior; the one test that verifies this function's concurrency opts back
    // in via `OPENHUMAN_RUN_LOGIN_GATED_SERVICES_IN_TEST`.
    if cfg!(test) && std::env::var_os("OPENHUMAN_RUN_LOGIN_GATED_SERVICES_IN_TEST").is_none() {
        log::debug!("[services] login-gated services skipped under unit test");
        return;
    }

    spawn_integrations_cache_warm(config);

    let started = std::time::Instant::now();
    // (service label, task) pairs so a panic surfaced on join is attributed to
    // the specific stage rather than an anonymous "a service failed".
    let mut tasks: Vec<(&'static str, tokio::task::JoinHandle<()>)> = Vec::new();

    // 1. Local AI — a read-only probe of the user's own runtime endpoint
    //    (`bootstrap` never spawns a runtime or pulls a model). Kept off the
    //    critical path so a slow endpoint cannot delay the other services.
    {
        let config = config.clone();
        tasks.push((
            "local_ai",
            tokio::spawn(async move {
                if config.local_ai.runtime_enabled {
                    let step = std::time::Instant::now();
                    log::debug!("[services] local AI endpoint probe starting");
                    let runtime = crate::inference::local_runtime_config(&config);
                    crate::inference::host_runtime::global(&config)
                        .bootstrap(&runtime)
                        .await;
                    log::debug!(
                        "[services] local AI endpoint probed after login ({} ms)",
                        step.elapsed().as_millis()
                    );
                } else {
                    log::debug!("[services] local AI disabled — skipping bootstrap");
                }
            }),
        ));
    }

    let total = tasks.len();
    let mut failed = 0usize;
    for (name, task) in tasks {
        if let Err(err) = task.await {
            failed += 1;
            log::warn!("[services] login-gated service '{name}' panicked during startup: {err}");
        }
    }
    let elapsed_ms = started.elapsed().as_millis();
    if failed == 0 {
        log::info!(
            "[services] all {total} login-gated services started concurrently ({elapsed_ms} ms)"
        );
    } else {
        log::warn!(
            "[services] {failed}/{total} login-gated services failed to start ({elapsed_ms} ms)"
        );
    }
}

/// Stop all login-gated background services.  Called from `clear_credential()`
/// on logout so orphan processes don't consume resources.
pub async fn stop_credential_gated_services(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
) {
    // 4. Local AI — reset state to idle. OpenHuman never owns the local
    //    runtime process (the user runs Ollama / LM Studio / MLX), so there is
    //    nothing to stop; we only clear the cached probe verdict so the
    //    endpoint is re-probed on next login.
    if config.local_ai.runtime_enabled {
        let service = crate::inference::host_runtime::global(config);
        let runtime = crate::inference::local_runtime_config(config);
        service.reset_to_idle(&runtime);
        log::info!("[services] local AI reset to idle on logout");
    }

    if let Err(error) = crate::integrations::composio::trigger_history::close_global(ctx).await {
        log::warn!("[composio][history] archive release failed on sign-out: {error}");
    }

    log::info!("[services] all login-gated services stopped");
}
