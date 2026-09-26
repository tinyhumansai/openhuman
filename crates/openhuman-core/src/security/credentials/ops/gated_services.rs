//! Login-gated background services (local AI, voice) started on session
//! store and stopped on session clear, plus the embedder-host detection
//! that gates global (operator-wide) activation side effects.

use crate::config::Config;

/// Whether this dispatch is running under an embedder-hosted core (the library
/// `Harness`) rather than the desktop shell or CLI.
///
/// An embedder supplies its own scoped [`Config`] via `CoreBuilder::config`,
/// so its `auth_store_session` must keep activation and credential state under
/// that config's path and never touch the operator's global
/// `~/.openhuman/active_user.toml` / `users/` tree.
pub(super) fn is_embedder_host() -> bool {
    crate::core::runtime::context::CoreContext::current_embedder_config().is_some()
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

/// Start all login-gated background services (local AI and voice). Called both
/// from the initial boot path (when an existing
/// session is detected) and from `set_credential()` when a credential is installed.
pub async fn start_credential_gated_services(config: &Config) {
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
    // include detached, long-lived continuous audio capture that outlives the
    // test that spawned it and interleaves with the
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

    // An embedder's task-local user-root policy does not cross spawn_blocking;
    // its first agent build warms the correctly scoped cache instead.
    if !is_embedder_host() {
        let skills_workspace = config.workspace_dir.clone();
        // Detached on purpose: the warm-up must not delay startup.
        tokio::task::spawn_blocking(move || {
            let count = crate::skills::load_workflow_metadata(&skills_workspace).len();
            log::debug!("[services] skill metadata cache warmed entries={count}");
        });
    }

    spawn_integrations_cache_warm(config);

    let started = std::time::Instant::now();
    // (service label, task) pairs so a panic surfaced on join is attributed to
    // the specific stage rather than an anonymous "a service failed".
    let mut tasks: Vec<(&'static str, tokio::task::JoinHandle<()>)> = Vec::new();

    // 1. Local AI (Ollama, embeddings) — the heaviest single warm-up,
    //    so keeping it off the critical path for the others is the biggest win.
    {
        let config = config.clone();
        tasks.push((
            "local_ai",
            tokio::spawn(async move {
                if config.local_ai.runtime_enabled {
                    let step = std::time::Instant::now();
                    log::debug!("[services] local AI bootstrap starting");
                    let runtime = crate::inference::local_runtime_config(&config);
                    crate::inference::host_runtime::global(&config)
                        .bootstrap(&runtime)
                        .await;
                    log::debug!(
                        "[services] local AI bootstrapped after login ({} ms)",
                        step.elapsed().as_millis()
                    );
                } else {
                    log::debug!("[services] local AI disabled — skipping bootstrap");
                }
            }),
        ));
    }

    // 2+3. Voice hotkey services — the user-facing command registration. The
    //      embedded voice server owns the single rdev listener; the standalone
    //      dictation listener only starts when the server is NOT auto-starting,
    //      so keep these two ordered *relative to each other* (but concurrent
    //      with everything else).
    {
        let config = config.clone();
        tasks.push((
            "voice_hotkey",
            tokio::spawn(async move {
                let step = std::time::Instant::now();
                crate::voice::server::start_if_enabled(&config).await;
                if !config.voice_server.auto_start {
                    crate::voice::dictation_listener::start_if_enabled(&config).await;
                }
                log::debug!(
                    "[services] voice hotkey services registered ({} ms)",
                    step.elapsed().as_millis()
                );
            }),
        ));
    }

    // 3b. Always-on listening (Phase 2): continuous mic + VAD → STT → agent.
    //     Its cold WASAPI init (`always_on::spawn_capture_thread`) is the
    //     Windows-specific blocker; it now runs the blocking capture-readiness
    //     handshake on the blocking pool (see `always_on::start_if_enabled`), so
    //     on its own task it neither stalls an async worker nor the hotkey /
    //     command registration above.
    {
        let config = config.clone();
        tasks.push((
            "always_on",
            tokio::spawn(async move {
                let step = std::time::Instant::now();
                crate::voice::always_on::start_if_enabled(&config).await;
                log::debug!(
                    "[services] always-on listening started ({} ms)",
                    step.elapsed().as_millis()
                );
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
pub async fn stop_credential_gated_services(config: &Config) {
    // 2. Voice server
    if let Some(server) = crate::voice::server::try_global_server() {
        server.stop().await;
        log::info!("[services] voice server stopped on logout");
    }

    // 4. Local AI — reset state to idle. We don't kill the Ollama process
    //    (it may be serving other clients or mid-download), but we clear
    //    the internal state so it re-bootstraps on next login.
    if config.local_ai.runtime_enabled {
        let service = crate::inference::host_runtime::global(config);
        let runtime = crate::inference::local_runtime_config(config);
        service.reset_to_idle(&runtime);
        log::info!("[services] local AI reset to idle on logout");
    }

    // 5. Dictation listener — abort the hotkey forwarder task so it doesn't
    //    accumulate duplicate rdev listeners across logout → login cycles.
    crate::voice::dictation_listener::stop();

    // 6. Always-on listening — disable the runtime gate so the mic capture loop
    //    stops transcribing/delivering after logout (no audio processed while
    //    logged out). Symmetric with start_login_gated_services step 3b.
    crate::voice::always_on::stop();

    log::info!("[services] all login-gated services stopped");
}

/// Historical names, kept for call sites outside this module.
pub use start_credential_gated_services as start_login_gated_services;
pub use stop_credential_gated_services as stop_login_gated_services;
