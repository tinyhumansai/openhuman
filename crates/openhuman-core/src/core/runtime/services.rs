//! Background service spawns.
//!
//! Extracted (Phase 0 — pure motion) from the inline `tokio::spawn` blocks that
//! used to live in `run_server_inner` (`crates/openhuman-core/src/core/jsonrpc.rs`, ~lines 2050-2191).
//! Each function spawns one long-lived background service as a detached task,
//! preserving the exact gating and behavior of the original inline block.
//!
//! Today these are launched unconditionally from `run_server_inner`; the
//! per-service *config* gates (`config.cron.enabled`, `config.heartbeat.enabled`,
//! `OPENHUMAN_DISABLE_CHANNEL_LISTENERS`, `has_listening_integrations()`) stay
//! inside each function. Phase 1 lifts the *selection* (should this service be
//! spawned at all) up to a `ServiceSet` chosen by the embedder, while these
//! functions keep their config gates (is it enabled for this user).

// `MemoryHostConfig` was imported here for `config.to_arc()`, the argument to
// the engine's `queue::start`. That call left with the in-process engine
// (openhuman#5560 — see `start_bootstrap_jobs`); every remaining `config.…` in
// this file is plain field access on OpenHuman's own `Config`.
use crate::config::Config;
use crate::core::runtime::ServiceSet;

/// Background bootstrap for login-gated services (local AI, voice, screen
/// intelligence, autocomplete) plus the subconscious engine + heartbeat.
///
/// Heavy services are only started when a user is logged in. If no user session
/// exists on disk, startup is deferred until the login handler in
/// `credentials::ops::store_session()` triggers it. The autocomplete shutdown
/// hook is registered unconditionally.
pub fn spawn_login_gated_services(embedded_core: bool) {
    tokio::spawn(async move {
        match crate::config::Config::load_or_init().await {
            Ok(config) => {
                if embedded_core {
                    log::debug!("[core] embedded core startup");
                } else {
                    log::debug!("[core] desktop core startup");
                }

                // Check if a user is already logged in from a previous session.
                let already_logged_in = crate::config::default_root_openhuman_dir()
                    .ok()
                    .and_then(|root| crate::config::read_active_user_id(&root))
                    .is_some();

                if already_logged_in {
                    // User has an active session — start all services now.
                    log::info!("[services] existing session found, starting services");
                    crate::security::credentials::ops::start_login_gated_services(&config).await;
                } else {
                    log::info!(
                        "[services] no active session — deferring service startup until login"
                    );
                }
            }
            Err(err) => {
                log::warn!("[core] config load failed, skipping service startup: {err}");
            }
        }
    });
}

/// Periodic self-update checker (default: every 1 hour).
pub fn spawn_update_scheduler() {
    tokio::spawn(async {
        match crate::config::Config::load_or_init().await {
            Ok(config) => {
                crate::platform::update::scheduler::run(config.update).await;
            }
            Err(err) => {
                log::warn!("[core] config load failed, skipping update scheduler: {err}");
            }
        }
    });
}

/// Boot-time flow-run reconciliation (bug B42): reconciles any `flow_runs` row
/// left at `running` by a prior process (crash/SIGKILL/power loss — where the
/// in-process `RunRowFinalizer` drop-guard never got to run) to a terminal
/// `interrupted`, so the run-details sidebar never shows a perpetual blank
/// spinner for a run nothing is executing.
///
/// Owned by the flows domain rather than piggybacked on cron bootstrap: runs
/// can be started by the RPC "Run" control, the agent `run_flow` tool and the
/// trigger bus, none of which need the cron *service* to be in the active
/// [`ServiceSet`]. Gating this on cron would silently skip reconciliation on any
/// cron-less selection (`headless_api()`, embedders), leaving prior-process
/// orphans wedged forever. Selected by the `flows` **domain** flag instead, and
/// safe at any point in boot — the sweep's own `PROCESS_RUN_FLOOR` guard means
/// it can never touch a run this process started, so it carries no ordering
/// requirement against the cron scheduler or any agent turn.
pub fn spawn_flows_boot_reconcile() {
    #[cfg(feature = "flows")]
    {
        log::debug!("[flows] boot reconcile: scheduling orphaned-run sweep");
        tokio::spawn(async {
            log::debug!("[flows] boot reconcile: loading config");
            match crate::config::Config::load_or_init().await {
                Ok(config) => {
                    let swept =
                        crate::flows::ops::sweep_orphaned_running_runs_on_boot(&config).await;
                    // Logged unconditionally: a silent success and a task that
                    // never ran are otherwise indistinguishable in a boot log.
                    log::debug!("[flows] boot reconcile: completed; reconciled_runs={swept}");
                    if swept > 0 {
                        log::info!(
                            "[flows] boot sweep reconciled {swept} orphaned running run(s) to 'interrupted'"
                        );
                    }
                }
                Err(err) => {
                    log::warn!("[core] config load failed, skipping flows boot reconcile: {err}");
                }
            }
        });
    }
    #[cfg(not(feature = "flows"))]
    log::debug!("[flows] flows feature disabled at compile time — no boot run reconciliation");
}

/// Cron scheduler — polls `due_jobs()` every ~5s and executes them
/// automatically. Gated by `config.cron.enabled`.
pub fn spawn_cron_service() {
    tokio::spawn(async {
        match crate::config::Config::load_or_init().await {
            Ok(config) => {
                if !config.cron.enabled {
                    log::info!("[cron] scheduler disabled via config; skipping");
                    return;
                }
                log::info!("[cron] spawning scheduler polling loop");
                // Re-register the cron job for every enabled, schedule-trigger
                // flow (issue B2) — idempotent, so a flow whose binding
                // predates this feature (or was otherwise lost) gets its
                // schedule re-registered without the user re-toggling it.
                // Gated with flows — absent entirely from a slim build.
                #[cfg(feature = "flows")]
                if let Err(e) =
                    crate::flows::ops::reconcile_schedule_triggers_on_boot(&config).await
                {
                    log::warn!(
                        "[flows] boot reconciliation of schedule-trigger cron jobs failed: {e}"
                    );
                }
                if let Err(e) = crate::cron::scheduler::run(config).await {
                    log::error!("[cron] scheduler loop ended with error: {e}");
                }
            }
            Err(err) => {
                log::warn!("[core] config load failed, skipping cron scheduler: {err}");
            }
        }
    });
}

/// Realtime channel listeners (Telegram getUpdates, Discord gateway, etc.).
///
/// Without this task, `openhuman run` would only expose RPC while inbound bot
/// messages are never polled. Skipped entirely when
/// `OPENHUMAN_DISABLE_CHANNEL_LISTENERS` is set to `1`/`true`, and returns early
/// when no channel integrations are configured.
pub fn spawn_channels_service() {
    // Compile-time `channels` gate: the body names `channels::start_channels`,
    // so the whole thing is `#[cfg]`-gated. With the feature off there are no
    // realtime listeners to spawn.
    #[cfg(feature = "channels")]
    if std::env::var("OPENHUMAN_DISABLE_CHANNEL_LISTENERS")
        .ok()
        .filter(|s| s == "1" || s.eq_ignore_ascii_case("true"))
        .is_none()
    {
        // Capture before loading config: logout during that await must also
        // invalidate a listener that has not finished starting yet.
        let channel_session = crate::channels::session::channel_session();
        tokio::spawn(async move {
            let config = match crate::config::Config::load_or_init().await {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("[channels] could not load config for listeners: {e}");
                    return;
                }
            };
            if !config.channels_config.has_listening_integrations() {
                log::debug!(
                    "[channels] no channel integrations configured; not spawning listeners"
                );
                return;
            }
            log::info!("[channels] spawning in-process realtime listeners (Telegram, Discord, …)");
            if let Err(e) =
                crate::channels::start_channels_with_session(config, channel_session).await
            {
                log::error!("[channels] start_channels ended with error: {e}");
            }
        });
    } else {
        log::info!("[channels] OPENHUMAN_DISABLE_CHANNEL_LISTENERS set — skipping start_channels");
    }
    #[cfg(not(feature = "channels"))]
    log::debug!("[channels] channels feature disabled at compile time — not spawning listeners");
}

/// Which bootstrap jobs a given [`ServiceSet`] enables — the single source of
/// truth for the flag→job mapping.
///
/// Computed by [`bootstrap_job_plan`] (a pure fn) so the wiring can be unit
/// tested without spawning the detached, global-state loops that
/// [`start_bootstrap_jobs`] launches. Each field maps 1:1 to one spawn site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BootstrapJobPlan {
    /// Memory queue ingestion workers (`memory_queue::start`).
    pub memory_queue: bool,
    /// Composio periodic connection sync (`composio::start_periodic_sync`).
    pub composio_integration_sync: bool,
    /// Workspace memory-source periodic sync — repos, folders, RSS, web pages
    /// (`memory_sync::workspace::start_workspace_periodic_sync`).
    pub workspace_memory_sync: bool,
    /// Proactive external task-source polling (`task_sources::start_periodic_poll`).
    pub task_source_pollers: bool,
    /// Eager native-module preload (`modules::boot::load_declared_modules`):
    /// the memory module resolving at boot, off the request path.
    pub module_preload: bool,
}

/// Pure flag→job mapping for [`start_bootstrap_jobs`]. No side effects.
///
/// Note the Composio integration sync AND the one-shot Composio source reconcile
/// both ride `services.integrations` — both no-op without active Composio
/// connections, so they share the one concern flag. `channels` gates NO
/// bootstrap job (its remaining meaning is exactly `spawn_channels_service`).
pub(crate) fn bootstrap_job_plan(services: &ServiceSet) -> BootstrapJobPlan {
    BootstrapJobPlan {
        memory_queue: services.memory_queue,
        composio_integration_sync: services.integrations,
        workspace_memory_sync: services.memory_sync,
        task_source_pollers: services.cron,
        // The memory module is the only eager module, so its preload is memory
        // background work and rides the same flag as the queue.
        module_preload: services.memory_queue,
    }
}

/// Starts legacy bootstrap loops that predate [`ServiceSet`].
///
/// These are separated from pure subscriber registration so a no-background
/// runtime can register handlers first without permanently suppressing a later
/// desktop/runtime-with-services boot.
///
/// Selection is computed once via [`bootstrap_job_plan`], then each job spawns
/// behind its own concern flag. The four non-channel jobs used to ride
/// `services.channels` — a channels-off + memory/integrations-on embedder
/// silently lost all of them (#5028) — so they now sit behind `integrations` /
/// `memory_sync` instead.
///
/// `config` feeds the native-module preload and nothing else here: the
/// engine's `queue::start` was this function's other consumer of it, and the
/// loaded TinyMemory module starts that pool itself now (openhuman#5560).
pub fn start_bootstrap_jobs(services: ServiceSet, config: &Config) {
    let plan = bootstrap_job_plan(&services);
    log::debug!("[runtime.bootstrap] starting bootstrap jobs with plan {plan:?}");

    // Native modules the registry marks eager — today TinyMemory, when the
    // memory driver is module-backed. Off the boot path: the first launch on a
    // machine downloads the release, and becoming RPC-ready must not wait on
    // the network. A warm launch maps the cached library in milliseconds, so
    // the first memory call finds it serving instead of starting the load
    // itself and waiting behind it.
    if plan.module_preload {
        spawn_module_preload(config);
    } else {
        log::debug!("[runtime.bootstrap] native module preload disabled by ServiceSet");
    }

    // ── The queue pool moved into the module, and must NOT be started here ──
    //
    // This block used to call `tinymemory_core::queue::start(config.to_arc())`,
    // and it was the only caller of the engine's worker pool in any tree.
    // openhuman#5560 deletes the host's second, in-process engine, so that call
    // has no engine to drain — and `tinymemory` v1.5.0's module starts its own
    // pool at load (`tinymemory-module/src/lib.rs`, `start_queue_pool`), which
    // is what keeps ingest, `retry_failed` and `ensure_reembed_backfill` alive.
    //
    // **Restoring the call would not be a duplicate, it would be a second pool
    // that cannot see the first.** The `cdylib` links its own copy of
    // `tinymemory-core`, so the `Once` inside `queue::start` is a *different*
    // static from the host's: two pools would claim jobs from one SQLite queue
    // with neither aware of the other. That is why the line is gone rather than
    // gated.
    //
    // `plan.memory_queue` is kept — it is `ServiceSet`'s statement of intent
    // and other jobs may hang off it — but the host has no work to do for it.
    if plan.memory_queue {
        log::debug!(
            "[runtime.bootstrap] memory queue workers are owned by the tinymemory module (start_queue_pool); host starts none"
        );
    } else {
        log::debug!("[runtime.bootstrap] memory queue workers disabled by ServiceSet");
    }

    // Integrations — Composio source reconcile. No-ops without active
    // connections.
    //
    // ── The periodic loops are NOT started here any more ────────────────────
    //
    // They were, and deleting them was blocked on upstream rather than on
    // taste: `start_periodic_sync` is not host code, it re-exported through
    // `integrations::composio` to `memory::sync::composio::periodic`, which was
    // `pub use tinymemory_core::sync::composio::*` — engine code running in
    // this process against the engine this host used to boot.
    //
    // tinymemory v1.6.0 moves both loops into the module and closes the three
    // things that stopped them working there: the cadence, the Composio mode,
    // and the module's client not being in the engine's global slot. The host
    // now passes the first two in `ModuleConfig` (see `modules::ops`).
    //
    // Restoring either call would be worse than a duplicate. The cdylib carries
    // its OWN copy of `tinymemory-core`, so each loop's `OnceLock` is a
    // different static from this process's: a host that starts them while
    // loading the module gets TWO pairs of loops over one store, and neither
    // can see the other.
    if plan.composio_integration_sync {
        log::debug!("[runtime.bootstrap] starting composio source reconcile");
        tokio::spawn(async {
            log::debug!("[runtime.bootstrap] composio source reconcile started");
            crate::memory::sources::reconcile::ensure_composio_sources().await;
            log::debug!("[runtime.bootstrap] composio source reconcile completed");
        });
    } else {
        log::debug!(
            "[runtime.bootstrap] composio integration sync + source reconcile disabled by ServiceSet"
        );
    }

    // Memory sync — workspace-kind memory sources (GitHub repos, folders, RSS,
    // web pages) get their own cadence loop; the Composio scheduler above only
    // walks Composio connections.
    if plan.workspace_memory_sync {
        // Owned by the module for the same reason as the Composio loop above.
        // The flag survives because `ServiceSet` is the host's declaration of
        // which background work it wants running at all, and a host that turns
        // this off should not have the module running it either — wiring that
        // through is follow-up, and until then this logs the divergence rather
        // than hiding it.
        log::debug!(
            "[runtime.bootstrap] workspace memory-source periodic sync is owned by the memory \
             module; this process starts none"
        );
    } else {
        log::debug!("[runtime.bootstrap] workspace periodic sync disabled by ServiceSet");
    }

    if plan.task_source_pollers {
        log::debug!("[runtime.bootstrap] starting task-source poller");
        crate::integrations::task_sources::start_periodic_poll();
    } else {
        log::debug!("[runtime.bootstrap] task-source polling disabled by ServiceSet");
    }

    log::debug!("[runtime.bootstrap] bootstrap job dispatch complete");
}

/// Resolve every eager native module in the background.
#[cfg(feature = "modules")]
fn spawn_module_preload(config: &Config) {
    let config = config.clone();
    tokio::spawn(async move {
        log::debug!("[runtime.bootstrap] native module preload started");
        crate::modules::boot::load_declared_modules(&config).await;
        log::debug!("[runtime.bootstrap] native module preload finished");
    });
}

/// Without the module host compiled in there is nothing to preload.
#[cfg(not(feature = "modules"))]
fn spawn_module_preload(_config: &Config) {
    log::debug!("[runtime.bootstrap] native module preload skipped: modules are compiled out");
}

/// Starts one-shot boot background work selected by [`ServiceSet`].
pub async fn start_boot_once_jobs(services: ServiceSet, config: &Config) {
    // The orphaned-run sweep does NOT live here. It runs in
    // `CoreBuilder::build`, which every runtime goes through — these jobs only
    // run from `serve()`, so a build-only embedder would never be swept.

    if services.harness_init {
        let cfg_for_init = config.clone();
        tokio::spawn(async move {
            crate::agent::harness_init::run_harness_init(cfg_for_init).await;
        });
    } else {
        log::debug!("[runtime] harness init disabled by ServiceSet");
    }

    if services.skill_catalog_refresh {
        crate::skills::catalog::ops::start_boot_catalog_refresh();
    } else {
        log::debug!("[runtime] boot catalog refresh disabled by ServiceSet");
    }

    if services.mcp_boot {
        // The MCP domain boots itself: service, installed-server reconnect
        // pass, and reconnect supervisor are orchestrated there, and it is
        // idempotent — normally a no-op, because `register_domain_subscribers`
        // brings the domain up when it enables it. Repeated here because the
        // two are gated separately: a `ServiceSet` that boots MCP is entitled
        // to a service whether or not the RPC domain was turned on.
        crate::mcp::start_boot_jobs(config);
    } else {
        log::debug!("[runtime] MCP boot-spawn disabled by ServiceSet");
        log::debug!("[runtime] MCP reconnect supervisor disabled by ServiceSet");
    }
}

/// Prunes retired scheduled jobs before a built runtime can expose the
/// scheduler to in-process or HTTP callers.
pub(crate) async fn run_legacy_migrations(config: &Config) {
    match crate::cron::seed::prune_retired_jobs(config) {
        Ok(count) if count > 0 => {
            log::info!("[cron] removed {count} retired autopilot job(s)");
        }
        Ok(_) => {}
        Err(e) => log::warn!("[cron] failed to prune retired jobs: {e}"),
    }
}

/// Auto-connect Socket.IO to the backend when enabled by the service selection.
pub fn spawn_socket_auto_connect(
    services: ServiceSet,
    socket_mgr: std::sync::Arc<crate::platform::socket::SocketManager>,
) {
    if services.socketio {
        tokio::spawn(async move {
            log::info!("[socket] Checking for stored session to auto-connect...");
            let config = match Config::load_or_init().await {
                Ok(c) => std::sync::Arc::new(c),
                Err(e) => {
                    log::debug!("[socket] Config not available for auto-connect: {e}");
                    return;
                }
            };
            // No TinyHumans connection (no backend transport installed): there
            // is no backend to hold a socket to, so skip quietly.
            if !crate::api::transport::is_installed() {
                log::debug!("[socket] No backend transport installed — skipping auto-connect");
                return;
            }
            let api_url = crate::api::config::effective_backend_api_url(&config.api_url);
            let initial_token = match crate::api::jwt::get_session_token(&config) {
                Ok(Some(t))
                    if crate::security::credentials::session_support::is_local_session_token(
                        &t,
                    ) =>
                {
                    // The offline local credential has no TinyHumans account,
                    // so the backend would only reject the handshake.
                    log::info!(
                        "[socket] Offline local session — skipping auto-connect (no hosted account)"
                    );
                    return;
                }
                Ok(Some(t)) => t,
                Ok(None) => {
                    log::info!(
                        "[socket] No session token stored — skipping auto-connect (will connect after login)"
                    );
                    return;
                }
                Err(e) => {
                    log::warn!("[socket] Failed to read session token: {e}");
                    return;
                }
            };
            log::info!(
                "[socket] Session token found — auto-connecting to {}",
                api_url
            );
            // Rebind the socket identity in one serialized transaction. The active profile may have
            // changed since CoreRuntime::build(), so the build-time Config is
            // not authoritative here.
            let _rebind = socket_mgr.lock_identity_rebind().await;
            // The renderer's `socket_connect_with_session` RPC connects the same
            // core to the same backend with the same token. Whichever path runs
            // second used to tear the other's live socket down and redo the
            // handshake (#6181); if it is already up for this identity there is
            // nothing to rebind.
            if socket_mgr.is_live_for(&api_url, &initial_token) {
                log::info!(
                    "[socket] Auto-connect: {api_url} already connected with this session — kept the socket"
                );
                return;
            }
            if let Err(e) = socket_mgr.disconnect().await {
                log::error!("[socket] Auto-connect could not stop the prior connection: {e}");
                return;
            }
            let provider =
                crate::platform::socket::token_provider::token_provider_from_config(config);
            if let Err(e) = socket_mgr.connect_with_provider(&api_url, provider).await {
                // Signing out between the token check above and the provider's
                // read leaves no token (Sentry 35911). That is a user-state
                // race, not a fault: warn so it stays a breadcrumb.
                if e.contains("no session token stored") {
                    log::warn!(
                        "[socket] Auto-connect skipped — session cleared before connect: {e}"
                    );
                } else {
                    log::error!("[socket] Auto-connect failed: {e}");
                }
            } else {
                log::info!("[socket] Auto-connect initiated successfully");
            }
        });
    } else {
        log::debug!("[socket] auto-connect disabled by ServiceSet");
    }
}

#[cfg(test)]
#[path = "services_tests.rs"]
mod tests;
