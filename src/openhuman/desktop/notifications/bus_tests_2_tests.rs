use super::*;

#[test]
fn cron_completed_produces_agents_notification() {
    let ev = DomainEvent::CronJobCompleted {
        job_id: "job-1".into(),
        success: true,
        output: "done".into(),
    };
    let n = event_to_notification(&ev).expect("should produce notification");
    assert_eq!(n.category, CoreNotificationCategory::Agents);
    assert_eq!(n.title, "Cron job completed");
    assert!(n.body.contains("job-1"));
}

#[test]
fn provider_api_key_rejected_produces_system_notification() {
    let ev = DomainEvent::ProviderApiKeyRejected {
        provider: "openrouter".into(),
        message: "openrouter rejected the API key (HTTP 401). Update your openrouter \
                  API key in Connections → API keys → LLM to restore it."
            .into(),
    };
    let n = event_to_notification(&ev).expect("should produce notification");
    assert_eq!(n.category, CoreNotificationCategory::System);
    assert_eq!(n.title, "API key rejected");
    assert!(n.body.contains("openrouter"));
    assert!(n.body.contains("Connections"));
    assert_eq!(n.deep_link.as_deref(), Some("/connections?tab=llm"));
    assert!(n.id.starts_with("provider-key-rejected:openrouter:"));
}

#[test]
fn cron_failed_uses_failure_title() {
    let ev = DomainEvent::CronJobCompleted {
        job_id: "job-1".into(),
        success: false,
        output: "error".into(),
    };
    let n = event_to_notification(&ev).unwrap();
    assert_eq!(n.title, "Cron job failed");
}

#[test]
fn successful_webhook_is_silent() {
    let ev = DomainEvent::WebhookProcessed {
        tunnel_id: "t".into(),
        skill_id: "s".into(),
        method: "POST".into(),
        path: "/p".into(),
        correlation_id: "c".into(),
        status_code: 200,
        elapsed_ms: 5,
        error: None,
    };
    assert!(event_to_notification(&ev).is_none());
}

#[test]
fn failed_webhook_produces_system_notification() {
    let ev = DomainEvent::WebhookProcessed {
        tunnel_id: "t".into(),
        skill_id: "skill-x".into(),
        method: "POST".into(),
        path: "/p".into(),
        correlation_id: "c".into(),
        status_code: 500,
        elapsed_ms: 12,
        error: Some("boom".into()),
    };
    let n = event_to_notification(&ev).unwrap();
    assert_eq!(n.category, CoreNotificationCategory::System);
    assert!(n.body.contains("skill-x"));
    assert!(n.body.contains("boom"));
}

#[test]
fn subagent_completed_produces_agents_notification() {
    let ev = DomainEvent::SubagentCompleted {
        parent_session: "p".into(),
        task_id: "t".into(),
        agent_id: "researcher".into(),
        elapsed_ms: 100,
        output_chars: 500,
        iterations: 3,
    };
    let n = event_to_notification(&ev).unwrap();
    assert_eq!(n.category, CoreNotificationCategory::Agents);
    assert!(n.body.contains("researcher"));
    assert!(n.body.contains("500"));
}

#[test]
fn subagent_failed_produces_agents_notification() {
    let ev = DomainEvent::SubagentFailed {
        parent_session: "p".into(),
        task_id: "t".into(),
        agent_id: "researcher".into(),
        error: "context window exceeded".into(),
    };
    let n = event_to_notification(&ev).unwrap();
    assert_eq!(n.category, CoreNotificationCategory::Agents);
    assert_eq!(n.title, "Sub-agent failed");
    assert!(n.body.contains("researcher"));
    assert!(n.body.contains("context window exceeded"));
}

#[test]
fn unrelated_events_return_none() {
    let ev = DomainEvent::AgentTurnCompleted {
        session_id: "s".into(),
        text_chars: 1,
        iterations: 1,
    };
    assert!(event_to_notification(&ev).is_none());
}

#[test]
fn notification_triaged_escalate_produces_agents_notification() {
    let ev = DomainEvent::NotificationTriaged {
        id: "n1".into(),
        provider: "slack".into(),
        action: "escalate".into(),
        importance_score: 0.9,
        latency_ms: 100,
        routed: true,
    };
    let n = event_to_notification(&ev).expect("should produce notification");
    assert_eq!(n.category, CoreNotificationCategory::Agents);
    assert!(n.body.contains("escalate"));
    assert!(n.deep_link.as_deref() == Some("/notifications"));
}

#[test]
fn notification_triaged_react_uses_follow_up_copy() {
    let ev = DomainEvent::NotificationTriaged {
        id: "n2".into(),
        provider: "discord".into(),
        action: "react".into(),
        importance_score: 0.7,
        latency_ms: 120,
        routed: true,
    };
    let n = event_to_notification(&ev).expect("should produce notification");
    assert_eq!(n.category, CoreNotificationCategory::Agents);
    assert!(n.body.contains("Routed for follow-up"));
}

#[test]
fn notification_triaged_drop_is_silent() {
    let ev = DomainEvent::NotificationTriaged {
        id: "n1".into(),
        provider: "gmail".into(),
        action: "drop".into(),
        importance_score: 0.1,
        latency_ms: 50,
        routed: false,
    };
    assert!(event_to_notification(&ev).is_none());
}

#[test]
fn notification_triaged_unrouted_escalate_is_silent() {
    let ev = DomainEvent::NotificationTriaged {
        id: "n1".into(),
        provider: "slack".into(),
        action: "escalate".into(),
        importance_score: 0.9,
        latency_ms: 100,
        routed: false,
    };
    assert!(event_to_notification(&ev).is_none());
}

// ── MCP reconnect supervisor (#5931) ────────────────────────────────────────

/// The workspace an MCP supervisor event is attributed to. The pure
/// translator ignores it — the bridge's `store_target` / `should_announce`
/// are what read
/// it — so these cases all use the same one.
fn mcp_workspace() -> std::path::PathBuf {
    std::path::PathBuf::from("/tmp/openhuman-ws")
}

#[test]
fn mcp_first_failed_reconnect_tells_the_user_tools_are_unavailable() {
    let ev = DomainEvent::McpServerReconnectFailed {
        server_id: "srv-1".into(),
        qualified_name: "ac.inference.sh/mcp".into(),
        error: "mcp transport failure for `https://api.inference.sh`: connection reset".into(),
        failures: 1,
        retry_in_secs: 5,
        workspace_dir: mcp_workspace(),
    };
    let n = event_to_notification(&ev).expect("the first failure of an episode notifies");
    assert_eq!(n.category, CoreNotificationCategory::System);
    assert_eq!(n.title, "MCP server unavailable");
    assert!(n.body.contains("ac.inference.sh/mcp"), "{}", n.body);
    // Deliberately NOT "retrying in 5s". `retry_in_secs` is tinymcp's backoff
    // and is faithful on the event, but this host drives `Supervisor::tick`
    // from its own 60s interval, so the backoff only decides eligibility on the
    // next tick — a "5s" banner sits there for a minute. Pinned as an absence
    // as well as a presence, so the false precision cannot come back.
    assert!(!n.body.contains("retrying in"), "{}", n.body);
    assert!(n.body.contains("retries automatically"), "{}", n.body);
    assert!(n.body.contains("connection reset"), "{}", n.body);
    assert_eq!(n.deep_link.as_deref(), Some("/connections?tab=mcp"));
    assert!(n.id.starts_with("mcp-unavailable:srv-1:"));
}

#[test]
fn mcp_later_failed_reconnects_stay_quiet() {
    // The backoff retries every few minutes for as long as the server is
    // down; the user heard about it once, on the first failure.
    let ev = DomainEvent::McpServerReconnectFailed {
        server_id: "srv-1".into(),
        qualified_name: "ac.inference.sh/mcp".into(),
        error: "connection refused".into(),
        failures: 2,
        retry_in_secs: 10,
        workspace_dir: mcp_workspace(),
    };
    assert!(event_to_notification(&ev).is_none());
}

#[test]
fn mcp_recovery_after_failures_is_announced() {
    let ev = DomainEvent::McpServerReconnected {
        server_id: "srv-1".into(),
        qualified_name: "ac.inference.sh/mcp".into(),
        tool_count: 25,
        after_failures: 2,
        workspace_dir: mcp_workspace(),
    };
    let n = event_to_notification(&ev).expect("a server that had stayed down coming back notifies");
    assert_eq!(n.category, CoreNotificationCategory::System);
    assert_eq!(n.title, "MCP server reconnected");
    assert!(n.body.contains("25 tools"), "{}", n.body);
    assert!(n.body.contains("2 failed attempt"), "{}", n.body);
    assert_eq!(n.deep_link.as_deref(), Some("/connections?tab=mcp"));
    assert!(n.id.starts_with("mcp-restored:srv-1:"));
}

#[test]
fn mcp_rebuild_within_the_same_tick_is_not_a_notification() {
    // The common field case: one request failed, the session was rebuilt a
    // second later, nobody noticed. Event Log only.
    let ev = DomainEvent::McpServerReconnected {
        server_id: "srv-1".into(),
        qualified_name: "ac.inference.sh/mcp".into(),
        tool_count: 25,
        after_failures: 0,
        workspace_dir: mcp_workspace(),
    };
    assert!(event_to_notification(&ev).is_none());
}

#[test]
fn mcp_parked_server_tells_the_user_how_to_recover() {
    let ev = DomainEvent::McpServerParked {
        server_id: "srv-1".into(),
        qualified_name: "@modelcontextprotocol/server-github".into(),
        error: "the `uvx` launcher is not installed".into(),
        workspace_dir: mcp_workspace(),
    };
    let n = event_to_notification(&ev).expect("a parked server notifies");
    assert_eq!(n.category, CoreNotificationCategory::System);
    assert_eq!(n.title, "MCP server can't start");
    assert!(
        n.body.contains("@modelcontextprotocol/server-github"),
        "{}",
        n.body
    );
    assert!(n.body.contains("uvx"), "{}", n.body);
    assert!(n.body.contains("disable and re-enable"), "{}", n.body);
    assert_eq!(n.deep_link.as_deref(), Some("/connections?tab=mcp"));
    assert!(n.id.starts_with("mcp-parked:srv-1:"));
}

#[test]
fn mcp_probe_timeouts_and_transport_drops_are_event_log_only() {
    let timed_out = DomainEvent::McpServerProbeTimedOut {
        server_id: "srv-1".into(),
        qualified_name: "ac.inference.sh/mcp".into(),
        probe_timeout_secs: 8,
        consecutive_timeouts: 1,
        teardown_after: 3,
        workspace_dir: mcp_workspace(),
    };
    let dropped = DomainEvent::McpServerTransportDropped {
        server_id: "srv-1".into(),
        qualified_name: "ac.inference.sh/mcp".into(),
        outcome: "broken".into(),
        detail: Some("connection reset".into()),
        elapsed_ms: Some(1961),
        consecutive_timeouts: 0,
        workspace_dir: mcp_workspace(),
    };
    assert!(event_to_notification(&timed_out).is_none());
    assert!(event_to_notification(&dropped).is_none());
}

// ── Workspace routing (#5931) ───────────────────────────────────────────────
//
// One process supervises every workspace it has opened, and this bridge is
// registered once with the workspace that booted. A supervisor event is
// therefore stored under the workspace it names — not this bridge's, which
// would file one account's outage in another's inbox, and not nowhere, which
// would silence the workspace the user is actually in once the boot binding
// is the stale one.

fn bridge_for(workspace: &std::path::Path) -> NotificationBridgeSubscriber {
    NotificationBridgeSubscriber::new(config_for(workspace))
}

fn config_for(workspace: &std::path::Path) -> crate::openhuman::config::Config {
    let mut config = crate::openhuman::config::Config::default();
    config.workspace_dir = workspace.to_path_buf();
    config
}

fn parked_in(workspace: &std::path::Path) -> DomainEvent {
    DomainEvent::McpServerParked {
        server_id: "srv-1".into(),
        qualified_name: "ac.inference.sh/mcp".into(),
        error: "the `uvx` launcher is not installed".into(),
        workspace_dir: workspace.to_path_buf(),
    }
}

#[test]
fn an_event_from_this_workspace_uses_this_bridge_s_own_store() {
    let workspace = std::path::Path::new("/tmp/openhuman-ws-a");
    let config = config_for(workspace);
    let target = bridge_for(workspace).store_target(&config, &parked_in(workspace));
    assert_eq!(target.workspace_dir, workspace);
}

#[test]
fn an_event_from_another_workspace_is_filed_under_that_workspace() {
    let own = std::path::Path::new("/tmp/openhuman-ws-a");
    let other = std::path::Path::new("/tmp/openhuman-ws-b");
    let config = config_for(own);
    let target = bridge_for(own).store_target(&config, &parked_in(other));
    assert_eq!(
        target.workspace_dir, other,
        "the event's workspace decides the store, not the bridge's binding"
    );
}

#[test]
fn every_supervisor_variant_is_routed_not_only_the_notifying_ones() {
    let own = std::path::Path::new("/tmp/openhuman-ws-a");
    let config = config_for(own);
    let bridge = bridge_for(own);
    let other = mcp_workspace();
    let foreign = [
        DomainEvent::McpServerProbeTimedOut {
            server_id: "srv-1".into(),
            qualified_name: "ac.inference.sh/mcp".into(),
            probe_timeout_secs: 8,
            consecutive_timeouts: 1,
            teardown_after: 3,
            workspace_dir: other.clone(),
        },
        DomainEvent::McpServerTransportDropped {
            server_id: "srv-1".into(),
            qualified_name: "ac.inference.sh/mcp".into(),
            outcome: "broken".into(),
            detail: None,
            elapsed_ms: None,
            consecutive_timeouts: 0,
            workspace_dir: other.clone(),
        },
        DomainEvent::McpServerReconnected {
            server_id: "srv-1".into(),
            qualified_name: "ac.inference.sh/mcp".into(),
            tool_count: 25,
            after_failures: 2,
            workspace_dir: other.clone(),
        },
        DomainEvent::McpServerReconnectFailed {
            server_id: "srv-1".into(),
            qualified_name: "ac.inference.sh/mcp".into(),
            error: "connection refused".into(),
            failures: 1,
            retry_in_secs: 5,
            workspace_dir: other.clone(),
        },
        parked_in(&other),
    ];
    for event in &foreign {
        assert_eq!(
            bridge.store_target(&config, event).workspace_dir,
            other,
            "{} should be routed to its own workspace",
            event.variant_name()
        );
    }
}

#[test]
fn an_event_that_names_no_workspace_uses_this_bridge_s_store() {
    // Every variant this bridge handled before #5931 — a cron job, a webhook,
    // a rejected API key — is process-wide and keeps this bridge's store.
    let own = std::path::Path::new("/tmp/openhuman-ws-a");
    let config = config_for(own);
    let target = bridge_for(own).store_target(
        &config,
        &DomainEvent::CronJobCompleted {
            job_id: "job-1".into(),
            success: true,
            output: "done".into(),
        },
    );
    assert_eq!(target.workspace_dir, own);
}

/// End to end through `handle`, across a workspace switch: the bridge is bound
/// to A, the user has moved to B, and B's supervisor is what reports. B's
/// outage must reach B's store — not A's, and not nowhere.
#[tokio::test]
async fn an_outage_is_filed_under_its_own_workspace_not_the_bridge_s() {
    use crate::openhuman::desktop::notifications::store;
    use tempfile::TempDir;

    let booted = TempDir::new().unwrap();
    let switched_to = TempDir::new().unwrap();
    let booted_config = config_for(booted.path());
    let switched_config = config_for(switched_to.path());
    let bridge = NotificationBridgeSubscriber::new(booted_config.clone());

    bridge.handle(&parked_in(switched_to.path())).await;

    let landed = store::list_core_notifications(&switched_config, true, 50).unwrap();
    assert_eq!(
        landed.len(),
        1,
        "the active workspace's outage must not be dropped"
    );
    assert!(landed[0].id.starts_with("mcp-parked:srv-1:"));
    assert!(
        store::list_core_notifications(&booted_config, true, 50)
            .unwrap()
            .is_empty(),
        "and must not land in the workspace the bridge happens to be bound to"
    );

    // The bridge's own workspace still works the way it always did.
    bridge.handle(&parked_in(booted.path())).await;
    assert_eq!(
        store::list_core_notifications(&booted_config, true, 50)
            .unwrap()
            .len(),
        1
    );
}

// ── Live announcement gating (#5931) ────────────────────────────────────────
//
// Routing the *store* by the event's workspace is only half the answer: the
// live path has no per-client routing at all (`core::socketio` emits
// `core_notification` to every connected client), so an outage from a
// workspace the user has switched away from would raise a banner naming that
// workspace's server and its error inside the account they are in.

/// RAII guard for `OPENHUMAN_WORKSPACE`, which is the first thing
/// `config::active_workspace_dir` consults — so it is how a test says which
/// workspace is the active one. Mirrors the guard in
/// `config::workspace::ops_tests`; must be held with `TEST_ENV_LOCK`.
struct ActiveWorkspaceEnvGuard;

impl ActiveWorkspaceEnvGuard {
    fn set(path: &std::path::Path) -> Self {
        // SAFETY: caller holds `TEST_ENV_LOCK`, so no other thread in this
        // process is reading or mutating this env var.
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", path);
        }
        Self
    }
}

impl Drop for ActiveWorkspaceEnvGuard {
    fn drop(&mut self) {
        // SAFETY: same contract as `set` — the lock is held for the whole test.
        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }
}

#[tokio::test]
async fn the_active_workspace_s_outage_is_announced() {
    // Held for the whole test. NOT dropped explicitly: `_guard` is declared
    // after it, so scope exit destroys the guard first and the env var is
    // cleared while this lock is still held. Releasing the lock early would
    // let the next test set its own override and have this guard erase it.
    let _lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let active = tempfile::TempDir::new().unwrap();
    let _guard = ActiveWorkspaceEnvGuard::set(active.path());

    // Ask the resolver what it made of the override rather than assuming:
    // `OPENHUMAN_WORKSPACE` names a *config* root, and which subdirectory of
    // it is the workspace is the loader's rule, not this test's.
    let resolved = crate::openhuman::config::active_workspace_dir()
        .await
        .expect("the override resolves");
    let bridge = bridge_for(&resolved);
    assert!(
        matches!(
            bridge.should_announce(&parked_in(&resolved)).await,
            super::Announce::Active(_)
        ),
        "the workspace the user is in must still hear about its own outage"
    );
}

#[tokio::test]
async fn a_switched_away_workspace_s_outage_is_not_announced() {
    // Held for the whole test. NOT dropped explicitly: `_guard` is declared
    // after it, so scope exit destroys the guard first and the env var is
    // cleared while this lock is still held. Releasing the lock early would
    // let the next test set its own override and have this guard erase it.
    let _lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let active = tempfile::TempDir::new().unwrap();
    let switched_away = tempfile::TempDir::new().unwrap();
    let _guard = ActiveWorkspaceEnvGuard::set(active.path());

    let resolved = crate::openhuman::config::active_workspace_dir()
        .await
        .expect("the override resolves");
    assert_ne!(resolved, switched_away.path(), "the fixture must differ");

    // The bridge's own binding is deliberately the stale one here — the gate
    // must key off the *live* workspace, not off what the bridge was built
    // with, or it would reproduce the staleness it exists to avoid.
    let bridge = bridge_for(switched_away.path());
    assert!(
        matches!(
            bridge
                .should_announce(&parked_in(switched_away.path()))
                .await,
            super::Announce::Suppressed
        ),
        "another workspace's server name and error must not reach this one"
    );
}

#[tokio::test]
async fn a_process_wide_event_is_announced_without_consulting_the_workspace() {
    // No env guard and no lock: an event that names no workspace returns on
    // the first line, before any resolution, so it cannot be affected by
    // whichever workspace happens to be active.
    let bridge = bridge_for(std::path::Path::new("/tmp/openhuman-ws-a"));
    assert!(matches!(
        bridge
            .should_announce(&DomainEvent::CronJobCompleted {
                job_id: "job-1".into(),
                success: true,
                output: "done".into(),
            })
            .await,
        super::Announce::Unbound
    ));
}

/// The announcement rule, including the arm that is otherwise reachable only
/// by making the on-disk config unreadable.
#[test]
fn the_announcement_rule_fails_closed_on_an_unknown_workspace() {
    let a = std::path::Path::new("/tmp/openhuman-ws-a");
    let b = std::path::Path::new("/tmp/openhuman-ws-b");

    // Not workspace-bound: never this rule's business.
    assert!(announces_to(None, Some(a)));
    assert!(announces_to(None, None));

    // Bound and live: announced only where it belongs.
    assert!(announces_to(Some(a), Some(a)));
    assert!(!announces_to(Some(a), Some(b)));

    // Bound, but the active workspace is unknown. Fail closed: the
    // notification is already persisted, so suppressing costs a banner, while
    // announcing could put another account's server name and error in front of
    // whoever is connected.
    assert!(!announces_to(Some(a), None));
}
