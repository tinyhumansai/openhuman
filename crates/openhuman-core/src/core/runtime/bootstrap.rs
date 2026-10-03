//! Long-lived runtime infrastructure brought up once per core boot.
//!
//! [`bootstrap_core_runtime`] runs from
//! [`CoreContext::init`](super::context::CoreContext::init) whether or not the
//! runtime ever binds a listener; [`start_core_runtime_services`] runs from
//! [`CoreRuntime::start_services`](super::CoreRuntime::start_services)
//! once a transport has bound, or directly when there is none.

/// Initializes long-lived socket/event-bus infrastructure.
///
/// `host_kind` identifies the embedding process (Tauri desktop shell vs
/// standalone CLI / Docker). It drives the approval-gate's host-aware
/// decision tree: under the Tauri shell, the `OPENHUMAN_APPROVAL_GATE=0`
/// env override is ignored and a domain event is published so the UI can
/// surface a banner; under CLI / Docker the override is honored (with a
/// noisy log + a domain event so any connected dashboard can flag it).
pub(crate) async fn bootstrap_core_runtime(
    host_kind: crate::core::types::HostKind,
    config: Option<crate::config::Config>,
    domains: crate::core::runtime::DomainSet,
) {
    use crate::platform::socket::{set_global_socket_manager, SocketManager};
    use std::sync::Arc;
    // `embedded_core` derived from host_kind so the rest of the function (which
    // already keys behavior off the boolean) stays unchanged.
    let embedded_core = host_kind.is_desktop_shell();
    let Some(mut cfg) = config else {
        log::error!(
            "[runtime] Config unavailable for runtime bootstrap; workspace-bound startup skipped"
        );
        return;
    };
    let workspace_dir = cfg.workspace_dir.clone();

    // --- Event bus bootstrap ---
    // Ensure the global event bus is initialized (no-op if already done by start_channels).
    crate::core::bus::init().await.expect("bus init");
    let agent_enabled = domains.allows(crate::core::all::DomainGroup::Agent);
    if agent_enabled {
        // Host decision: `OPENHUMAN_FILE_STATE_GUARD=0|false|off|no` turns the
        // cross-agent file-staleness guard off.
        let guard_disabled = std::env::var("OPENHUMAN_FILE_STATE_GUARD")
            .map(|v| matches!(v.as_str(), "0" | "false" | "off" | "no"))
            .unwrap_or(false);
        if guard_disabled {
            log::debug!("[file_state] guard disabled via OPENHUMAN_FILE_STATE_GUARD");
        }
        tinytools_std::file_state::init_global(!guard_disabled);
    } else {
        log::debug!("[boot] agent file-state coordinator SKIPPED — Agent domain disabled");
    }
    // Register domain subscribers for cross-module event handling. Ungated infra
    // runs once (INFRA: Once) and each DomainGroup installs at most once via the
    // per-group `group_first_time` set, so repeated calls to
    // bootstrap_core_runtime() cannot double-subscribe (and a later, wider
    // DomainSet still installs its newly-enabled groups).
    super::subscribers::register_domain_subscribers(
        workspace_dir.clone(),
        cfg.clone(),
        embedded_core,
        domains,
    );

    // Latch the config-corruption recovery signal (#5167) so `app_state_snapshot`
    // keeps reporting it after the loader heals the file on this same boot; the
    // frontend raises a one-shot "settings were reset" notice off it.
    crate::desktop::app_state::latch_from_config(&cfg);

    // --- Configurable hooks -------------------------------------------
    // Read every `hooks.json` layer and, only if something is configured,
    // install the harness bridge. Boot is the right moment: a hook that is
    // meant to gate the first tool call of the first turn has to be loaded
    // before any session exists, and the alternative — loading lazily on the
    // first event — would let that first call through while the file is read.
    crate::hooks::init(&cfg).await;

    // --- Turn-state recovery -------------------------------------------
    // Any per-thread turn snapshots left on disk from a previous process
    // are stale by definition — there is no live driver to resume them.
    // Stamp them as `Interrupted` so the UI can offer a retry without
    // confusing a stale `Streaming` lifecycle for an in-flight turn.
    {
        let now = chrono::Utc::now().to_rfc3339();
        match tinyagents_session::turn_state::store::mark_all_interrupted(
            workspace_dir.clone(),
            &now,
        ) {
            Ok(0) => {}
            Ok(count) => {
                log::info!("[runtime] marked {count} stale turn snapshot(s) as interrupted")
            }
            Err(err) => {
                log::warn!("[runtime] failed to mark stale turn snapshots interrupted: {err}")
            }
        }
    }

    // --- Run-ledger recovery -------------------------------------------
    // Detached sub-agent runs (`spawn_async_subagent`) from a previous process
    // are gone with that process. Any `agent_runs` row still marked `running`
    // at boot is orphaned — its driver died without firing a terminal event, so
    // the finalizer never settled it. Stamp such rows `interrupted` so they stop
    // rendering as perpetual "running" timeline entries on thread reopen.
    if agent_enabled {
        match tinyagents_session::run_ledger::interrupt_orphaned_agent_runs(&cfg.workspace_dir) {
            Ok(0) => {}
            Ok(count) => log::info!("[runtime] settled {count} orphaned agent run(s) on startup"),
            Err(err) => log::warn!("[runtime] failed to settle orphaned agent runs: {err}"),
        }

        // --- Detached sub-agent TaskStore reconciliation -------------------
        // The durable orchestration TaskStore (`<workspace>/.openhuman/
        // orchestration_tasks.jsonl`) can hold non-terminal sub-agent records left
        // by a previous process — their detached executor (abort handle +
        // cooperative CancellationToken) died with that process, so they cannot be
        // re-attached. Reconcile each orphan to a terminal state and emit the typed
        // terminal lifecycle event so the run ledger finalizes. Best-effort and
        // non-fatal (issue #4249 / 07.2 steps 2 & 4).
        let reconciled =
            crate::agent::orchestration::running_subagents::reconcile_orphaned_tasks_on_boot(
                &workspace_dir,
            );
        if reconciled > 0 {
            log::info!(
                "[runtime] reconciled {reconciled} orphaned detached sub-agent task(s) on startup"
            );
        }
    } else {
        log::debug!(
            "[boot] agent run-ledger + orchestration task reconciliation SKIPPED — Agent domain disabled"
        );
    }

    // --- Cost dashboard tracker ---
    // Activates the previously-dormant CostTracker so the dashboard RPC
    // surface (`openhuman.cost_get_dashboard`) and `record_provider_usage`
    // share one JSONL-backed store. Idempotent.
    crate::platform::cost::init_global(cfg.cost.clone(), &workspace_dir);

    // --- x402 payment ledger ---
    // Initializes the JSONL-backed spending ledger for machine-payable API
    // payments (x402 protocol). Budget defaults can be overridden via
    // the `openhuman.x402_update_budget` RPC. Gated on the Web3 domain (#4808
    // review): under `harness()`/`none()` the x402 controllers are absent, so
    // their ledger must not initialize either.
    if domains.allows(crate::core::all::DomainGroup::Web3) {
        let x402_session = format!("x402-{}", uuid::Uuid::new_v4());
        crate::web3::x402::init_ledger(&workspace_dir, &x402_session);
    } else {
        log::debug!("[boot] x402 payment ledger SKIPPED — Web3 domain disabled");
    }

    // --- Sub-agent definition registry bootstrap ---
    // Loads built-in archetype definitions plus any custom TOML files
    // under `<workspace>/agents/*.toml`. Idempotent — safe to call
    // multiple times. Uses the per-user scoped workspace_dir.
    if agent_enabled {
        if let Err(err) =
            crate::agent::harness::AgentDefinitionRegistry::init_global(&workspace_dir)
        {
            log::warn!(
                "[runtime] AgentDefinitionRegistry::init_global failed: {err} — \
                 spawn_subagent will be unavailable until restart"
            );
        }
    } else {
        log::debug!("[boot] agent definition registry SKIPPED — Agent domain disabled");
    }

    // --- Agent sandbox + projects dirs ---
    // Create the action sandbox + default projects home and register the
    // projects dir as a ReadWrite trusted root BEFORE building the live policy
    // below (so the trusted root is reflected in `from_config`). This is the
    // always-run boot for web-chat-only desktop cores; without it a fresh
    // install with no messaging integrations leaves `~/OpenHuman/projects`
    // uncreated and every shell-tool `current_dir` fails with ERROR_DIRECTORY
    // (os error 267) on Windows / ENOENT on Unix (#3353, RC-A). Idempotent — a
    // later `start_channels` calls the same helper.
    crate::config::ensure_agent_dirs(&mut cfg).await;

    // --- Live SecurityPolicy ---
    // Install the process-global live policy on the always-run serve boot, not
    // only inside `start_channels` (which is skipped for web-chat-only cores
    // with no messaging integrations). Without this, `live_policy::current()`
    // would be empty on those cores, so the ApprovalGate's `auto_approve`
    // allowlist and `config.update_autonomy_settings` reloads (`reload_from`)
    // would be inert until a session with integrations starts. `from_config`
    // injects the default projects root, so this matches what `start_channels`
    // installs; idempotent — a later `start_channels` re-installs an equivalent
    // policy.
    let action_dir = cfg.action_dir.clone();
    crate::security::live_policy::install(
        std::sync::Arc::new(
            crate::security::SecurityPolicy::from_config(
                &cfg.autonomy,
                &workspace_dir,
                &action_dir,
            )
            .with_privacy_mode(cfg.privacy.mode),
        ),
        workspace_dir.clone(),
        action_dir,
    );

    // --- Triggered-workflow subscriber ---
    // Install on the always-run serve boot, not only inside `start_channels`
    // (skipped for web-chat-only cores with no messaging integrations, and when
    // `OPENHUMAN_DISABLE_CHANNEL_LISTENERS=1`). Without this, any workflow
    // declaring `triggers:` was silently ignored on web-chat-only desktop
    // installs. Idempotent — shares a process-global OnceLock with the
    // `start_channels` site so it registers exactly once regardless of which
    // path runs first. (Matching only for now; activation handoff still pending.)
    // Gated on the Skills domain (#4808 review): under `harness()`/`none()` the
    // skills controllers are absent, so their trigger subscriber must not install.
    if domains.allows(crate::core::all::DomainGroup::Skills) {
        crate::skills::bus::ensure_triggered_workflow_subscriber(&workspace_dir);
    } else {
        log::debug!("[boot] triggered-workflow subscriber SKIPPED — Skills domain disabled");
    }

    // --- Approval gate (#1339) ---
    // ON by default; opt out with `OPENHUMAN_APPROVAL_GATE=0` (or `false`).
    // Prompt-class `external_effect()` tool calls route through
    // `ApprovalGate::intercept` and park until the UI dispatches
    // `approval_decide` (or the 10-minute TTL elapses → deny). Safe to default
    // on now that the release surface exists (ApprovalRequestCard + the Agent
    // OS access panel) AND only *interactive chat* turns park — background /
    // triage / cron turns carry no chat context and pass straight through, so
    // autonomous automation is never blocked.
    //
    // Host-aware override evaluation: under the Tauri desktop shell the env
    // override is treated as advisory only — the gate ALWAYS installs and a
    // `DomainEvent::ApprovalGateOverrideIgnored` is published so the UI can
    // surface a one-shot banner explaining the override was rejected. Under
    // standalone CLI / Docker (env-as-config is the operator's chosen
    // surface) the override is honored, but a `DomainEvent::ApprovalGateDisabled`
    // is still published so any connected dashboard / log shipper can
    // surface the elevated-privilege state.
    let env_override_requested = std::env::var("OPENHUMAN_APPROVAL_GATE")
        .map(|v| {
            let t = v.trim();
            t == "0" || t.eq_ignore_ascii_case("false")
        })
        .unwrap_or(false);
    let decision =
        crate::core::types::approval_gate_boot_decision(host_kind, env_override_requested);
    // Record the boot decision before publishing the warning event so the
    // first poll of `approval_get_gate_state` after boot reflects the same
    // host-aware verdict the event itself describes — no race.
    crate::security::approval::gate::record_boot_state(
        crate::security::approval::gate::ApprovalGateBootState {
            installed: decision.install_gate,
            disabled_by_env: decision.gate_disabled_by_override,
            override_ignored: decision.override_ignored,
            host: match host_kind {
                crate::core::types::HostKind::TauriShell => "tauri-shell",
                crate::core::types::HostKind::Cli => "cli",
                crate::core::types::HostKind::Docker => "docker",
                crate::core::types::HostKind::Library => "library",
            },
        },
    );
    if decision.override_ignored {
        log::warn!(
            "[runtime] OPENHUMAN_APPROVAL_GATE=0 IGNORED under desktop shell — \
             gate is always on for the Tauri host (host={})",
            host_kind.tag()
        );
        crate::core::bus::BUS.publish(
            crate::core::events::DomainEvent::ApprovalGateOverrideIgnored {
                host: host_kind.tag().to_string(),
            },
        );
    }
    // Bridge interactive web-surface events to the frontend: ApprovalRequested →
    // `approval_request` AND PlanReviewRequested → `plan_review_request` (both
    // handled by the same subscriber). Registered UNCONDITIONALLY here on the
    // always-run serve boot — the plan-review gate is independent of the approval
    // gate and parks turns even when `OPENHUMAN_APPROVAL_GATE=0`, while
    // `start_channels` is skipped for web-chat-only cores. Without this an
    // unguarded standalone/CLI/Docker core would park a plan review that never
    // reaches the UI and dies at the gate TTL. Idempotent (Once-guarded).
    crate::web_chat::register_approval_surface_subscriber();
    // Egress-surface bridge (privacy epic S2, #4436) — registered
    // unconditionally alongside the approval surface so external-transfer
    // disclosures reach the UI even on cores that skip `start_channels` or run
    // with the approval gate disabled. Idempotent (OnceLock-guarded).
    crate::web_chat::register_egress_surface_subscriber();
    // Agent-surface bridge (goals/todos/queue, C3) — registered unconditionally
    // for the same reason as the two bridges above: this JSON-RPC serve boot
    // path can run without `start_channels`. Idempotent (OnceLock-guarded).
    crate::web_chat::register_agent_surface_subscriber();

    if decision.install_gate {
        // Per-launch correlation token for the approval gate. This is
        // a fresh UUID every boot — it is NOT derived from the
        // JSON-RPC bearer (`OPENHUMAN_CORE_TOKEN` / the in-memory
        // auth subsystem) and carries no credential material, so it
        // is safe to log, persist, and surface in audit events.
        // `approval_list_pending` is session-agnostic so pending rows
        // from prior launches remain visible after restart; only the
        // per-session audit grouping changes across launches.
        let session_id = format!("session-{}", uuid::Uuid::new_v4());
        let _ =
            crate::security::approval::ApprovalGate::init_global(cfg.clone(), session_id.clone());
        log::info!(
            "[runtime] approval gate installed (on by default; set OPENHUMAN_APPROVAL_GATE=0 to disable, session_id={session_id}) — \
             Prompt-class external-effect tool calls park for approval in interactive chat turns"
        );
        // (The approval/plan-review surface bridge is registered unconditionally
        // above — it must run even when this gate-install branch is skipped.)
        crate::web_chat::register_artifact_surface_subscriber();
    } else {
        log::info!(
            "[runtime] approval gate DISABLED (OPENHUMAN_APPROVAL_GATE=0 honored on host={}) — \
             Prompt-class external-effect tool calls run unprompted",
            host_kind.tag()
        );
        crate::core::bus::BUS.publish(crate::core::events::DomainEvent::ApprovalGateDisabled {
            host: host_kind.tag().to_string(),
            reason: "env-override".to_string(),
        });
    }
    // Artifact surface bridges DomainEvent::ArtifactReady/Failed onto the web
    // channel ("Files in this chat" panel + ArtifactCard updates). This is
    // independent of the approval-gate config — keep it outside the
    // `if approval_gate` block so artifact events still publish when the user
    // sets OPENHUMAN_APPROVAL_GATE=0 (CR #3328947323 on PR #3026). Idempotent
    // (OnceLock-guarded inside register_artifact_surface_subscriber).
    crate::web_chat::register_artifact_surface_subscriber();
    // Memory-activity surface bridges DomainEvent::MemoryStored/Recalled onto
    // the web channel's `memory_activity` event (C5) — same unconditional
    // placement rationale as the bridges above. Idempotent (OnceLock-guarded).
    crate::web_chat::register_memory_activity_surface_subscriber();

    // --- Workspace migrations --------------------------------------------
    crate::platform::startup::run_workspace_migrations(&workspace_dir);

    // --- Socket manager bootstrap ---
    let socket_mgr = Arc::new(SocketManager::new());
    set_global_socket_manager(socket_mgr.clone());
    log::info!("[socket] SocketManager initialized and registered globally");
}

/// Starts selected background jobs after the runtime has entered `serve()`.
///
/// This deliberately sits outside [`bootstrap_core_runtime`]: embedders may call
/// `CoreBuilder::build()` only to use in-process RPC, and a failed listener bind
/// must not leave pollers, one-shot jobs, MCP processes, or socket reconnect work
/// running without a live runtime.
pub(crate) async fn start_core_runtime_services(
    services: crate::core::runtime::ServiceSet,
    config: Option<&crate::config::Config>,
) {
    let Some(cfg) = config else {
        log::error!(
            "[runtime] Config unavailable for runtime service startup; selected services skipped"
        );
        return;
    };

    // Long-lived bootstrap loops selected by ServiceSet.
    // One-time first-run initialization (managed Python runtime, Kompress,
    // managed Node runtime). Spawned AFTER subscribers are live but does NOT
    // block the ready signal — the core becomes RPC-ready immediately and the
    // frontend watches per-step progress via `openhuman.harness_init_status`.
    // On a warm host every step's `is_done` probe passes and this settles
    // instantly. See `crate::agent::harness_init`.
    crate::core::runtime::services::start_boot_once_jobs(services, cfg).await;

    // Long-lived bootstrap loops selected by ServiceSet. These start only
    // after the boot-once jobs above have completed.
    crate::core::runtime::services::start_bootstrap_jobs(services, cfg);

    match crate::platform::socket::global_socket_manager() {
        Some(socket_mgr) => {
            crate::core::runtime::services::spawn_socket_auto_connect(services, socket_mgr.clone());
        }
        None => {
            log::warn!(
                "[socket] SocketManager unavailable during runtime service startup; auto-connect skipped"
            );
        }
    }
}
