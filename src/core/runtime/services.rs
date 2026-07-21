//! Background service spawns.
//!
//! Extracted (Phase 0 — pure motion) from the inline `tokio::spawn` blocks that
//! used to live in `run_server_inner` (`src/core/jsonrpc.rs`, ~lines 2050-2191).
//! Each function spawns one long-lived background service as a detached task,
//! preserving the exact gating and behavior of the original inline block.
//!
//! Today these are launched unconditionally from `run_server_inner`; the
//! per-service *config* gates (`config.cron.enabled`, `config.heartbeat.enabled`,
//! `OPENHUMAN_DISABLE_CHANNEL_LISTENERS`, `has_listening_integrations()`) stay
//! inside each function. Phase 1 lifts the *selection* (should this service be
//! spawned at all) up to a `ServiceSet` chosen by the embedder, while these
//! functions keep their config gates (is it enabled for this user).

use std::sync::Once;

use crate::core::runtime::ServiceSet;
use crate::openhuman::config::Config;

/// Background bootstrap for login-gated services (local AI, voice, screen
/// intelligence, autocomplete) plus the subconscious engine + heartbeat.
///
/// Heavy services are only started when a user is logged in. If no user session
/// exists on disk, startup is deferred until the login handler in
/// `credentials::ops::store_session()` triggers it. The autocomplete shutdown
/// hook is registered unconditionally.
pub fn spawn_login_gated_services(embedded_core: bool) {
    tokio::spawn(async move {
        match crate::openhuman::config::Config::load_or_init().await {
            Ok(config) => {
                if embedded_core {
                    log::debug!("[core] embedded core startup");
                } else {
                    log::debug!("[core] desktop core startup");
                }

                // Register autocomplete shutdown hook so the engine (and its
                // Swift overlay helper) are stopped cleanly on process exit.
                // This is unconditional — the hook should fire regardless of
                // whether the user is currently logged in.
                crate::core::shutdown::register(|| async {
                    let engine = crate::openhuman::autocomplete::global_engine();
                    let status = engine.status().await;
                    if status.running {
                        log::info!(
                            "[core] stopping autocomplete engine (phase={})",
                            status.phase
                        );
                        engine.stop(None).await;
                        log::info!("[core] autocomplete engine stopped");
                    }
                });

                // Check if a user is already logged in from a previous session.
                let already_logged_in = crate::openhuman::config::default_root_openhuman_dir()
                    .ok()
                    .and_then(|root| crate::openhuman::config::read_active_user_id(&root))
                    .is_some();

                if already_logged_in {
                    // User has an active session — start all services now.
                    log::info!("[services] existing session found, starting services");
                    crate::openhuman::credentials::ops::start_login_gated_services(&config).await;

                    // Subconscious engine + heartbeat.
                    if !config.heartbeat.enabled {
                        log::info!("[subconscious] disabled by config (heartbeat.enabled = false)");
                    } else {
                        match crate::openhuman::subconscious::registry::bootstrap_after_login()
                            .await
                        {
                            Ok(()) => {
                                log::info!(
                                    "[subconscious] bootstrapped on startup (existing session)"
                                )
                            }
                            Err(e) => log::warn!("[subconscious] startup bootstrap failed: {e}"),
                        }
                    }
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
        match crate::openhuman::config::Config::load_or_init().await {
            Ok(config) => {
                crate::openhuman::update::scheduler::run(config.update).await;
            }
            Err(err) => {
                log::warn!("[core] config load failed, skipping update scheduler: {err}");
            }
        }
    });
}

/// Cron scheduler — polls `due_jobs()` every ~5s and executes them
/// automatically. Gated by `config.cron.enabled`.
pub fn spawn_cron_service() {
    tokio::spawn(async {
        match crate::openhuman::config::Config::load_or_init().await {
            Ok(config) => {
                if !config.cron.enabled {
                    log::info!("[cron] scheduler disabled via config; skipping");
                    return;
                }
                log::info!("[cron] spawning scheduler polling loop");
                // Ensure proactive agent jobs (e.g. the autonomous bounty job)
                // exist for already-onboarded users upgrading from a build that
                // predates them — otherwise their Settings toggle stays hidden.
                // Idempotent; no-op until onboarding is complete.
                if let Err(e) = crate::openhuman::cron::seed::seed_proactive_agents_on_boot(&config)
                {
                    log::warn!("[cron] boot seed of proactive agent jobs failed: {e}");
                }
                // Re-register the cron job for every enabled, schedule-trigger
                // flow (issue B2) — idempotent, so a flow whose binding
                // predates this feature (or was otherwise lost) gets its
                // schedule re-registered without the user re-toggling it.
                // Gated with flows — absent entirely from a slim build.
                #[cfg(feature = "flows")]
                if let Err(e) =
                    crate::openhuman::flows::ops::reconcile_schedule_triggers_on_boot(&config).await
                {
                    log::warn!(
                        "[flows] boot reconciliation of schedule-trigger cron jobs failed: {e}"
                    );
                }
                if let Err(e) = crate::openhuman::cron::scheduler::run(config).await {
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
        tokio::spawn(async move {
            let config = match crate::openhuman::config::Config::load_or_init().await {
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
            if let Err(e) = crate::openhuman::channels::start_channels(config).await {
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
    /// Orchestration relay-mailbox drain supervisor
    /// (`orchestration::start_message_drain_supervisor`).
    pub orchestration_drain: bool,
    /// Proactive task pollers (`task_sources::start_periodic_poll` +
    /// `agent::task_dispatcher::start_board_poller`).
    pub proactive_task_pollers: bool,
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
        orchestration_drain: services.orchestration,
        proactive_task_pollers: services.cron,
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
/// `memory_sync` / `orchestration` instead.
pub fn start_bootstrap_jobs(services: ServiceSet, config: &Config) {
    let plan = bootstrap_job_plan(&services);

    if plan.memory_queue {
        crate::openhuman::memory_queue::start(config.clone());
    } else {
        log::debug!("[runtime] memory queue workers disabled by ServiceSet");
    }

    // Integrations — Composio periodic connection sync + one-shot source
    // reconcile. Both no-op without active Composio connections.
    if plan.composio_integration_sync {
        crate::openhuman::composio::start_periodic_sync();
        tokio::spawn(async {
            crate::openhuman::memory_sources::reconcile::ensure_composio_sources().await;
        });
    } else {
        log::debug!("[composio] integration sync + source reconcile disabled by ServiceSet");
    }

    // Memory sync — workspace-kind memory sources (GitHub repos, folders, RSS,
    // web pages) get their own cadence loop; the Composio scheduler above only
    // walks Composio connections.
    if plan.workspace_memory_sync {
        crate::openhuman::memory_sync::workspace::start_workspace_periodic_sync();
    } else {
        log::debug!("[memory_sync] workspace periodic sync disabled by ServiceSet");
    }

    // Orchestration — relay-mailbox drain supervisor.
    if plan.orchestration_drain {
        crate::openhuman::orchestration::start_message_drain_supervisor();
    } else {
        log::debug!("[orchestration] message drain supervisor disabled by ServiceSet");
    }

    if plan.proactive_task_pollers {
        crate::openhuman::task_sources::start_periodic_poll();
        crate::openhuman::agent::task_dispatcher::start_board_poller();
    } else {
        log::debug!("[runtime] bootstrap proactive task pollers disabled by ServiceSet");
    }
}

/// Starts one-shot boot background work selected by [`ServiceSet`].
pub fn start_boot_once_jobs(services: ServiceSet, config: &Config) {
    if services.harness_init {
        let cfg_for_init = config.clone();
        tokio::spawn(async move {
            crate::openhuman::harness_init::run_harness_init(cfg_for_init).await;
        });
    } else {
        log::debug!("[runtime] harness init disabled by ServiceSet");
    }

    if services.skill_catalog_refresh {
        crate::openhuman::skill_registry::ops::start_boot_catalog_refresh();
    } else {
        log::debug!("[runtime] boot catalog refresh disabled by ServiceSet");
    }

    if services.mcp_boot {
        let cfg_for_mcp = config.clone();
        tokio::spawn(async move {
            crate::openhuman::mcp_registry::boot::spawn_installed_servers(&cfg_for_mcp).await;
        });
        spawn_mcp_reconnect_supervisor(config.clone());
    } else {
        log::debug!("[runtime] MCP boot-spawn disabled by ServiceSet");
        log::debug!("[runtime] MCP reconnect supervisor disabled by ServiceSet");
    }
}

fn spawn_mcp_reconnect_supervisor(config: Config) {
    static SUPERVISOR_SPAWNED: Once = Once::new();
    SUPERVISOR_SPAWNED.call_once(|| {
        tokio::spawn(async move {
            crate::openhuman::mcp_registry::supervisor::run(config).await;
        });
    });
}

/// Auto-connect Socket.IO to the backend when enabled by the service selection.
pub fn spawn_socket_auto_connect(
    services: ServiceSet,
    socket_mgr: std::sync::Arc<crate::openhuman::socket::SocketManager>,
) {
    if services.socketio {
        tokio::spawn(async move {
            log::info!("[socket] Checking for stored session to auto-connect...");
            let config = match Config::load_or_init().await {
                Ok(c) => c,
                Err(e) => {
                    log::debug!("[socket] Config not available for auto-connect: {e}");
                    return;
                }
            };
            let api_url = crate::api::config::effective_backend_api_url(&config.api_url);
            let token = match crate::api::jwt::get_session_token(&config) {
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
            if let Err(e) = socket_mgr.connect(&api_url, &token).await {
                log::error!("[socket] Auto-connect failed: {e}");
            } else {
                log::info!("[socket] Auto-connect initiated successfully");
            }
        });
    } else {
        log::debug!("[socket] auto-connect disabled by ServiceSet");
    }
}

#[cfg(test)]
mod tests {
    use super::{bootstrap_job_plan, BootstrapJobPlan};
    use crate::core::runtime::ServiceSet;

    /// desktop() must enable every bootstrap job — proves the un-bundling kept
    /// the desktop job set byte-identical.
    #[test]
    fn desktop_plan_enables_every_job() {
        let plan = bootstrap_job_plan(&ServiceSet::desktop());
        assert_eq!(
            plan,
            BootstrapJobPlan {
                memory_queue: true,
                composio_integration_sync: true,
                workspace_memory_sync: true,
                orchestration_drain: true,
                proactive_task_pollers: true,
            }
        );
    }

    /// none() / headless_api() run no bootstrap job at all.
    #[test]
    fn job_free_presets_enable_nothing() {
        let empty = BootstrapJobPlan {
            memory_queue: false,
            composio_integration_sync: false,
            workspace_memory_sync: false,
            orchestration_drain: false,
            proactive_task_pollers: false,
        };
        assert_eq!(bootstrap_job_plan(&ServiceSet::none()), empty);
        assert_eq!(bootstrap_job_plan(&ServiceSet::headless_api()), empty);
    }

    /// From none(), flipping exactly one concern flag enables exactly its job
    /// and nothing else.
    #[test]
    fn each_concern_flag_enables_exactly_its_job() {
        let mut integrations = ServiceSet::none();
        integrations.integrations = true;
        let plan = bootstrap_job_plan(&integrations);
        assert!(plan.composio_integration_sync);
        assert!(!plan.workspace_memory_sync);
        assert!(!plan.orchestration_drain);
        assert!(!plan.memory_queue);
        assert!(!plan.proactive_task_pollers);

        let mut memory_sync = ServiceSet::none();
        memory_sync.memory_sync = true;
        let plan = bootstrap_job_plan(&memory_sync);
        assert!(plan.workspace_memory_sync);
        assert!(!plan.composio_integration_sync);
        assert!(!plan.orchestration_drain);

        let mut orchestration = ServiceSet::none();
        orchestration.orchestration = true;
        let plan = bootstrap_job_plan(&orchestration);
        assert!(plan.orchestration_drain);
        assert!(!plan.composio_integration_sync);
        assert!(!plan.workspace_memory_sync);
    }

    /// From desktop(), disabling exactly one concern flag disables only its job.
    #[test]
    fn disabling_one_concern_disables_only_its_job() {
        let mut services = ServiceSet::desktop();
        services.integrations = false;
        let plan = bootstrap_job_plan(&services);
        assert!(!plan.composio_integration_sync);
        assert!(plan.workspace_memory_sync);
        assert!(plan.orchestration_drain);
        assert!(plan.memory_queue);
        assert!(plan.proactive_task_pollers);

        let mut services = ServiceSet::desktop();
        services.memory_sync = false;
        let plan = bootstrap_job_plan(&services);
        assert!(!plan.workspace_memory_sync);
        assert!(plan.composio_integration_sync);
        assert!(plan.orchestration_drain);

        let mut services = ServiceSet::desktop();
        services.orchestration = false;
        let plan = bootstrap_job_plan(&services);
        assert!(!plan.orchestration_drain);
        assert!(plan.composio_integration_sync);
        assert!(plan.workspace_memory_sync);
    }

    /// The #5028 regression: `channels` gates NO bootstrap job. Turning channels
    /// on by itself must enable zero sync jobs, and turning channels off while
    /// the new flags stay on must lose nothing.
    #[test]
    fn channels_flag_gates_no_bootstrap_job() {
        // channels=true alone → zero sync jobs.
        let mut channels_only = ServiceSet::none();
        channels_only.channels = true;
        let plan = bootstrap_job_plan(&channels_only);
        assert_eq!(
            plan,
            bootstrap_job_plan(&ServiceSet::none()),
            "channels alone must enable no bootstrap job"
        );

        // channels=false with every new flag on → identical to desktop's plan.
        let mut channels_off = ServiceSet::desktop();
        channels_off.channels = false;
        assert_eq!(
            bootstrap_job_plan(&channels_off),
            bootstrap_job_plan(&ServiceSet::desktop()),
            "dropping channels must not drop any bootstrap job"
        );
    }
}
