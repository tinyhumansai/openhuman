//! Process-wide event-bus subscriber registration for the core boot.
//!
//! [`register_domain_subscribers`] runs from
//! [`bootstrap_core_runtime`](super::bootstrap::bootstrap_core_runtime) on every
//! boot. Ungated infrastructure registers once per process; each gated
//! [`DomainGroup`](crate::core::all::DomainGroup) registers the first time a
//! boot enables it.

/// Per-`DomainGroup` gating decision for each event-bus subscriber that
/// [`register_domain_subscribers`] conditionally registers. Extracted as a
/// pure value so the subscriber→group mapping has a single source of truth
/// that the registrar consumes and tests assert directly — without registering
/// real subscribers or touching the process-global event bus (#4796 DoD item 3).
///
/// Unlisted subscribers (health, scheduler-gate, TokenJuice content-router,
/// session-token seeding, `SessionExpired`, service restart/shutdown) are
/// always registered as core/platform infra and intentionally absent here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DomainSubscriberPlan {
    /// Reserved for subscribers with no family of their own. Currently none:
    /// the reorg gave every subscriber that used to live here a real family
    /// (see the fields below), so this stays for future kernel-level ones.
    pub platform: bool,
    /// composio trigger archive + trigger subscriber + task-sources poller.
    pub integrations: bool,
    /// device tunnel handshake/peer-status subscriber.
    pub security: bool,
    /// notification bridge (desktop-shell delivery).
    pub desktop: bool,
    /// webhook request subscriber (skills own inbound webhook routing).
    pub skills: bool,
    /// channel-inbound + web-only proactive.
    pub channels: bool,
    /// flows trigger dispatch.
    pub flows: bool,
    /// memory v2: conversation ingestion, memory cron jobs, idle flusher.
    pub memory: bool,
    /// chat-thread persistence of channel turns (`threads::store`).
    pub threads: bool,
    /// agent handlers + background delivery + run-ledger finalizer + orchestration ingest.
    pub agent: bool,
    /// hosted orchestration ingest.
    pub hosted: bool,
    /// mcp::registry lifecycle bus init.
    pub mcp: bool,
}

impl DomainSubscriberPlan {
    /// The subscriber-registration plan for `domains`. Pure: no side effects.
    pub fn for_domains(domains: crate::core::runtime::DomainSet) -> Self {
        use crate::core::all::DomainGroup;
        Self {
            platform: domains.allows(DomainGroup::Platform),
            integrations: domains.allows(DomainGroup::Integrations),
            security: domains.allows(DomainGroup::Security),
            desktop: domains.allows(DomainGroup::Desktop),
            skills: domains.allows(DomainGroup::Skills),
            channels: domains.allows(DomainGroup::Channels),
            flows: domains.allows(DomainGroup::Flows),
            memory: domains.allows(DomainGroup::Memory),
            threads: domains.allows(DomainGroup::Threads),
            agent: domains.allows(DomainGroup::Agent),
            hosted: domains.allows(DomainGroup::Hosted),
            mcp: domains.allows(DomainGroup::Mcp),
        }
    }
}

/// Consume a domain's registration token only when the global event bus is
/// ready. An early bus-unavailable attempt therefore remains retryable.
fn group_first_time_when_bus_ready(
    completed: &std::sync::Mutex<std::collections::HashSet<crate::core::all::DomainGroup>>,
    group: crate::core::all::DomainGroup,
    bus_ready: bool,
) -> bool {
    if !bus_ready {
        log::warn!("[event_bus] deferred {group:?} subscriber registration - bus not initialized");
        return false;
    }

    completed
        .lock()
        .expect("domain-subscriber registry lock poisoned")
        .insert(group)
}

fn group_first_time(group: crate::core::all::DomainGroup) -> bool {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};

    static DONE: OnceLock<Mutex<HashSet<crate::core::all::DomainGroup>>> = OnceLock::new();
    group_first_time_when_bus_ready(
        DONE.get_or_init(|| Mutex::new(HashSet::new())),
        group,
        crate::core::bus::BUS.get().is_some(),
    )
}

/// Registers all long-lived domain event-bus subscribers, each group at most
/// once per process.
///
/// Ungated core/platform infra runs exactly once behind `INFRA: Once`; each
/// gated [`DomainGroup`](crate::core::all::DomainGroup) installs the first time
/// it is enabled after the event bus is ready, so widening the ambient
/// `DomainSet` on a later call (`harness()` → `full()`) still installs the
/// newly-enabled groups without double-subscribing the ones already registered.
pub(super) fn register_domain_subscribers(
    workspace_dir: std::path::PathBuf,
    config: crate::config::Config,
    embedded_core: bool,
    domains: crate::core::runtime::DomainSet,
) {
    use crate::core::all::DomainGroup;
    use std::sync::{Arc, Once};

    let plan = DomainSubscriberPlan::for_domains(domains);
    log::debug!("[event_bus] register_domain_subscribers: domains={domains:?} plan={plan:?}");

    // `subscribe_global` returns `None` only before the process-global bus is
    // initialized. Because that bus is a monotonic `OnceLock`, checking it here
    // guarantees the registrations in the guarded block cannot later lose bus
    // availability. A premature call leaves the group absent from `DONE`, so a
    // later bootstrap retries it instead of permanently skipping subscribers.

    // Seed the live tool-execution timeout from the persisted `[agent]` config
    // so a user-configured value (Settings → Agent OS access → Action timeout)
    // is in effect from the first tool call. `OPENHUMAN_TOOL_TIMEOUT_SECS`, when
    // set, still overrides this inside `set_tool_timeout_secs`. This lives on
    // the always-on boot path (not `channels::runtime::startup::start_channels`,
    // which is skipped for channel-less / web-chat-only cores) so the timeout
    // seeds for every install regardless of DomainSet — it is DomainSet-
    // independent process-global state, not a gated subscriber (#5027).
    //
    // Deliberately OUTSIDE the `INFRA: Once` below: `bootstrap_core_runtime`
    // re-runs on an in-process core restart (`CoreProcessHandle::restart` →
    // `ensure_running`, and `reset_local_data`/Clear-Local-Data) with a freshly
    // reloaded `Config`, but `INFRA` is already consumed — so a seed gated by it
    // would leave the process-global timeout pinned to the previous config's
    // value until Settings is re-saved. `set_tool_timeout_secs` is idempotent
    // (a plain atomic store honouring the env override), so re-seeding on every
    // call is correct and cheap and re-applies the current config each boot.
    let effective_timeout =
        crate::tools::timeout::set_tool_timeout_secs(config.agent.agent_timeout_secs);
    log::debug!(
        "[tool_timeout] seeded tool-execution timeout from config: configured={}s effective={}s",
        config.agent.agent_timeout_secs,
        effective_timeout
    );

    // Ungated core/platform infra — health, scheduler-gate, TokenJuice
    // content-router, session-token seeding, the SessionExpired handler, and
    // service restart/shutdown. These are DomainSet-independent, so they run
    // exactly once on the first call regardless of which composition boots
    // first. Registered BEFORE any gated subscriber so the SessionExpired
    // handler is live before a gated subscriber could publish a 401-derived
    // event. Leaked `SubscriptionHandle`s live for the whole process
    // (`SubscriptionHandle::drop` aborts the task).
    static INFRA: Once = Once::new();
    INFRA.call_once(|| {
        crate::platform::health::bus::register_health_subscriber();

        // Initialise the scheduler gate before any background AI workers start
        // so they observe a real policy on their first iteration (otherwise they
        // fall back to `Policy::Normal` and miss the initial throttle decision on
        // battery-powered hosts).
        crate::cron::scheduler_gate::init_global(config.scheduler_gate.clone());

        // A headless host (Docker / VPS / CI) has no interactive login; it
        // hands the core a credential through the environment instead. The
        // API key is a plain profile write, so it lands before the gate is
        // seeded from the store below; a session token runs the full
        // `set_credential` path and seeds the gate itself when it finishes.
        crate::security::credentials::seed_api_key_from_env(&config);
        if std::env::var_os(crate::security::credentials::BACKEND_SESSION_TOKEN_ENV).is_some() {
            let config = config.clone();
            tokio::spawn(async move {
                crate::security::credentials::seed_session_from_env(&config).await;
            });
        }

        // Seed the scheduler-gate signed-out override from the on-disk
        // credential — an API key (library runtime) or the app session.
        // Without this, a sidecar that boots with no stored credential would
        // happily spin up cron / channel loops and fire LLM requests that all
        // 401.
        if crate::security::credentials::api_key::has_api_key(&config) {
            log::info!("[auth] api-key credential present at startup — scheduler gate signed in");
            crate::cron::scheduler_gate::set_signed_out(false);
        } else {
            match crate::security::credentials::jwt::get_session_token(&config) {
                Ok(Some(_)) => {
                    crate::cron::scheduler_gate::set_signed_out(false);
                }
                Ok(None) => {
                    log::info!(
                        "[auth] no session token at startup — scheduler gate set to signed_out \
                         (config_path={}, keyring_backend={})",
                        config.config_path.display(),
                        crate::security::keyring::backend_name(),
                    );
                    crate::cron::scheduler_gate::set_signed_out(true);
                }
                Err(err) => {
                    log::warn!(
                        "[auth] failed to read session token at startup ({err}) — assuming signed_out \
                         (config_path={}, keyring_backend={})",
                        config.config_path.display(),
                        crate::security::keyring::backend_name(),
                    );
                    crate::cron::scheduler_gate::set_signed_out(true);
                }
            }
        }

        // Register the SessionExpired handler before any subscribers that might
        // publish 401-derived events, so the very first 401 is routed through
        // `clear_session` + the scheduler-gate override.
        if let Some(handle) = crate::core::bus::BUS.subscribe(Arc::new(
            crate::security::credentials::bus::SessionExpiredSubscriber::new(),
        )) {
            std::mem::forget(handle);
        } else {
            log::warn!(
                "[event_bus] failed to register SessionExpired subscriber — bus not initialized"
            );
        }

        // Restart requests go through a subscriber so every trigger path shares
        // the same respawn logic.
        crate::platform::service::bus::register_restart_subscriber();
        if embedded_core {
            log::info!(
                "[event_bus] embedded core: service shutdown subscriber not registered; Tauri cancellation token owns shutdown"
            );
        } else {
            // Shutdown requests use the same pattern; the standalone CLI
            // subscriber exits the current process after a short grace period.
            crate::platform::service::bus::register_shutdown_subscriber();
        }
    });

    // ---- Gated domain subscribers — each group installed at most once, the
    // first time its owning DomainGroup is enabled. -------------------------

    // Carved-out families: webhook (Skills), notification bridge (Desktop),
    // composio + task-sources (Integrations), and device tunnel (Security).
    if plan.skills {
        if group_first_time(DomainGroup::Skills) {
            if let Some(handle) = crate::core::bus::BUS.subscribe(Arc::new(
                crate::skills::webhooks::bus::WebhookRequestSubscriber::new(),
            )) {
                std::mem::forget(handle);
            } else {
                log::warn!(
                    "[event_bus] failed to register webhook subscriber — bus not initialized"
                );
            }
        }
    } else {
        log::debug!("[event_bus] webhook subscriber SKIPPED — Skills domain disabled");
    }

    if plan.desktop {
        if group_first_time(DomainGroup::Desktop) {
            crate::desktop::notifications::register_notification_bridge_subscriber(config.clone());
        }
    } else {
        log::debug!("[event_bus] notification bridge SKIPPED — Desktop domain disabled");
    }

    if plan.integrations {
        if group_first_time(DomainGroup::Integrations) {
            if let Err(error) =
                crate::integrations::composio::init_composio_trigger_history(workspace_dir.clone())
            {
                log::warn!("[composio][history] failed to initialize trigger archive: {error}");
            }
            crate::integrations::composio::register_composio_trigger_subscriber();
            crate::integrations::task_sources::bus::register_task_sources_subscriber();
        }
    } else {
        log::debug!(
            "[event_bus] composio + task-sources subscribers SKIPPED — Integrations domain disabled"
        );
    }

    if plan.security {
        if group_first_time(DomainGroup::Security) {
            // Device tunnel subscriber: handles tunnel:frame handshakes,
            // peer-status events, and register acks. Must be live before any
            // tunnel:frame events can arrive.
            crate::security::devices::bus::register_device_tunnel_subscriber();
        }
    } else {
        log::debug!("[event_bus] device-tunnel subscriber SKIPPED — Security domain disabled");
    }

    // Channels: inbound dispatch + web-only proactive messaging.
    // The `plan.channels` runtime guard cannot stand in for the compile-time
    // gate: the `channels::bus::ChannelInboundSubscriber` +
    // `channels::proactive` type paths below must still resolve for this to
    // compile, so the whole block is `#[cfg]`-gated too (mirrors the flows
    // dual-gate below).
    #[cfg(feature = "channels")]
    if plan.channels {
        if group_first_time(DomainGroup::Channels) {
            if let Some(handle) = crate::core::bus::BUS.subscribe(Arc::new(
                crate::channels::bus::ChannelInboundSubscriber::new(),
            )) {
                std::mem::forget(handle);
            } else {
                log::warn!(
                    "[event_bus] failed to register channel subscriber — bus not initialized"
                );
            }
            // Web-only proactive message subscriber (no external channel
            // instances are registered here in the desktop runtime).
            crate::channels::proactive::register_web_only_proactive_subscriber();
        }
    } else {
        log::debug!(
            "[event_bus] Channels subscribers (inbound + web-only proactive) SKIPPED — Channels domain disabled"
        );
    }
    #[cfg(not(feature = "channels"))]
    log::debug!(
        "[event_bus] Channels subscribers (inbound + web-only proactive) SKIPPED — channels feature disabled at compile time"
    );

    // Flows trigger dispatch (issue B2): maps FlowScheduleTick /
    // ComposioTriggerReceived / WebhookIncomingRequest onto enabled flows and
    // runs `flows::ops::flows_run`, so schedule/app-event workflows still
    // dispatch when no realtime channel is configured or
    // `OPENHUMAN_DISABLE_CHANNEL_LISTENERS` short-circuits `start_channels`.
    // The `plan.flows` runtime guard cannot stand in for the compile-time gate:
    // the `flows::bus::FlowTriggerSubscriber` type path below must still resolve
    // for this to compile, so the whole block is `#[cfg]`-gated too.
    #[cfg(feature = "flows")]
    if plan.flows {
        if group_first_time(DomainGroup::Flows) {
            if let Some(handle) = crate::core::bus::BUS.subscribe(Arc::new(
                crate::flows::bus::FlowTriggerSubscriber::new(Arc::new(config.clone())),
            )) {
                std::mem::forget(handle);
            } else {
                log::warn!(
                    "[event_bus] failed to register flows trigger subscriber — bus not initialized"
                );
            }
            // Post-run memory digest (issue #5173): on a successful
            // `FlowRunFinished`, writes a compact summary into the flow's own
            // private memory namespace so a later run can `flow_memory_recall`
            // it (e.g. a scheduled digest flow deduping what it already sent).
            // Registered in the same `group_first_time(DomainGroup::Flows)`
            // block as the trigger subscriber above — that guard only returns
            // `true` once per process, so a second, separate call here would
            // never register.
            if let Some(handle) = crate::core::bus::BUS.subscribe(Arc::new(
                crate::flows::bus::FlowRunDigestSubscriber::new(Arc::new(config.clone())),
            )) {
                std::mem::forget(handle);
            } else {
                log::warn!(
                    "[event_bus] failed to register flows run-digest subscriber — bus not initialized"
                );
            }
            // Dedup commit-on-success (issue #5263 PR2): on every terminal
            // `FlowRunFinished`, settles each `dedup` node in the flow's graph
            // — unions tentative keys into committed on success, releases
            // (clears) tentative on failure/cancel/interrupt. This is the
            // host half of the `dedup` node's exactly-once contract; the node
            // itself (`tinyflows::nodes::control_flow::dedup`) only ever
            // reads `committed` and writes `tentative`. Registered in the
            // same `group_first_time(DomainGroup::Flows)` block as the other
            // two flows subscribers above — see the digest subscriber's
            // comment just above for why a second `group_first_time` guard
            // here would be redundant.
            if let Some(handle) = crate::core::bus::BUS.subscribe(Arc::new(
                crate::flows::bus::DedupCommitSubscriber::new(Arc::new(config.clone())),
            )) {
                std::mem::forget(handle);
            } else {
                log::warn!(
                    "[event_bus] failed to register flows dedup-commit subscriber — bus not initialized"
                );
            }
        }
    } else {
        log::debug!("[event_bus] flows trigger subscriber SKIPPED — Flows domain disabled");
    }
    #[cfg(not(feature = "flows"))]
    log::debug!(
        "[event_bus] flows trigger subscriber SKIPPED — flows feature disabled at compile time"
    );

    // Threads: persist channel turns into the chat-thread store.
    if plan.threads {
        if group_first_time(DomainGroup::Threads) {
            crate::threads::store::register_conversation_persistence_subscriber(
                workspace_dir.clone(),
            );
        }
    } else {
        log::debug!(
            "[event_bus] conversation-persistence subscriber SKIPPED — Threads domain disabled"
        );
    }

    // Memory: conversation ingestion, memory cron jobs, idle flusher.
    if plan.memory {
        if group_first_time(DomainGroup::Memory) {
            crate::memory::register_memory_subscribers();
        }
    } else {
        log::debug!("[event_bus] memory subscribers SKIPPED — Memory domain disabled");
    }

    // Agent: native agent handlers + background-completion delivery +
    // run-ledger finalizer.
    if plan.agent {
        if group_first_time(DomainGroup::Agent) {
            // Native request handlers — the agent `agent.run_turn` handler is
            // what channel dispatch calls instead of importing
            // `run_tool_call_loop` directly.
            crate::agent::bus::register_agent_handlers();
            // Background-completion delivery: when a detached sub-agent
            // (spawn_async_subagent) finishes, surface its result back into the
            // originating chat as an idle-gated, batched, system-injected turn.
            crate::agent::orchestration::background_delivery::register_background_delivery();
            // Run-ledger finalizer: detached `spawn_async_subagent` runs outlive
            // their parent turn, so their terminal `AgentProgress` never reaches
            // the per-turn progress bridge that settles the ledger. This
            // global-bus subscriber settles `agent_runs` from
            // `DomainEvent::Subagent{Completed,Failed}`, preventing rows from
            // leaking as perpetual `running` timeline entries on thread reopen.
            crate::agent::orchestration::run_ledger_finalize::register_run_ledger_finalize_subscriber(&config);
        }
    } else {
        log::debug!(
            "[event_bus] agent handlers + background delivery + run-ledger finalizer SKIPPED — Agent domain disabled"
        );
    }

    // MCP clients lifecycle subscriber: logs McpServer{Installed,Connected,
    // Disconnected} + McpClientToolExecuted for observability. The boot-time
    // spawn of installed servers (boot::spawn_installed_servers) runs later in
    // bootstrap_core_runtime; this subscriber must be live before then so those
    // connect events are observed (issue #3039 gap A1).
    //
    // What bringing the domain up means is the domain's own; this only says
    // when. The service it opens is wanted here rather than later for the same
    // reason as the subscriber: every RPC handler in the domain reaches for it,
    // and opening it from the boot-connect job would leave a window where a
    // handler answers "still starting" to a caller whose domain is, as far as
    // anything else can tell, already up.
    if plan.mcp {
        if group_first_time(DomainGroup::Mcp) {
            crate::mcp::start(&config);
        }
    } else {
        log::debug!("[event_bus] mcp_registry bus init SKIPPED — Mcp domain disabled");
    }

    log::info!("[event_bus] domain subscriber registration complete: plan={plan:?}");
}

#[cfg(test)]
#[path = "subscribers_tests.rs"]
mod tests;
