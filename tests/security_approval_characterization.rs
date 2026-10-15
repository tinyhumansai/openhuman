//! Phase-0 approval characterization (issue #7328, plan Task 1) through the
//! public approval gate: park and decide, the 600s default and 180s copilot and
//! sub-agent park windows (read from the persisted `expires_at`), fail-closed
//! expiry, restart survival through the storage backend, cron-origin denial of
//! external effects, the `auto_approve_all` bypass (which writes no audit row
//! today) and per-flow trust.
//!
//! These pin current behaviour. Desired behaviour that does not exist yet is
//! written as a `desired_*` test that must panic with `DESIRED-RED` until the
//! tinysecurity module owns the gate; it is never deleted or blessed.

#[path = "support/env_guard.rs"]
mod env_guard;
#[path = "support/tinyhumans_boot.rs"]
mod tinyhumans_boot;

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use chrono::Utc;
use openhuman_core::{
    agent::turn_origin::{self, AgentTurnOrigin, TrustedAutomationSource},
    config::Config,
    security::{
        approval::{
            gate::ApprovalGate,
            is_unanswered_approval_reason, store,
            types::{ApprovalDecision, GateOutcome, PendingApproval},
            ApprovalChatContext, ApprovalTurnContext, FlowRunContext,
        },
        live_policy, SecurityPolicy,
    },
};
use serde_json::json;

struct Fixture {
    _dir: tempfile::TempDir,
    config: Config,
    gate: Arc<ApprovalGate>,
}

/// One gate per test process (the gate is a process singleton) over one scratch
/// workspace. Every test serialises on the env lock and uses a unique tool name.
fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        tinyhumans_boot::boot();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("OPENHUMAN_WORKSPACE", dir.path());
        std::env::set_var("OPENHUMAN_KEYRING_BACKEND", "file");
        std::env::remove_var("OPENHUMAN_STORAGE_URL");
        std::env::remove_var("OPENHUMAN_APPROVAL_GATE");
        std::env::remove_var("OPENHUMAN_APPROVAL_TTL_SECS");
        let mut config = Config::default();
        config.workspace_dir = dir.path().to_path_buf();
        config.config_path = dir.path().join("config.toml");
        config.action_dir = dir.path().join("action");
        let gate = ApprovalGate::init_global(config.clone(), "session-approval-characterization");
        Fixture {
            _dir: dir,
            config,
            gate,
        }
    })
}

fn web_origin() -> AgentTurnOrigin {
    AgentTurnOrigin::WebChat {
        thread_id: "t-char".into(),
        client_id: "c-char".into(),
        request_id: Some("req-char".into()),
    }
}

fn chat_ctx() -> ApprovalChatContext {
    ApprovalChatContext {
        thread_id: "t-char".into(),
        client_id: "c-char".into(),
        request_id: None,
    }
}

fn flow_origin(flow_id: &str, require_approval: bool) -> AgentTurnOrigin {
    AgentTurnOrigin::TrustedAutomation {
        job_id: flow_id.into(),
        source: TrustedAutomationSource::Workflow { require_approval },
    }
}

async fn wait_pending(gate: &ApprovalGate, tool: &str) -> PendingApproval {
    for _ in 0..500 {
        if let Some(p) = gate
            .list_pending()
            .unwrap()
            .into_iter()
            .find(|p| p.tool_name == tool)
        {
            return p;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no pending approval appeared for {tool}");
}

fn secs_until_expiry(p: &PendingApproval) -> i64 {
    (p.expires_at.expect("parked rows carry an expiry") - Utc::now()).num_seconds()
}

fn assert_window(p: &PendingApproval, expected: i64) {
    let left = secs_until_expiry(p);
    assert!(
        (expected - 30..=expected).contains(&left),
        "park window for {} should be about {expected}s, was {left}s",
        p.tool_name
    );
}

async fn park_and_approve(
    tool: &'static str,
    origin: AgentTurnOrigin,
    copilot: bool,
    with_ctx: bool,
) -> PendingApproval {
    let fx = fixture();
    let gate = fx.gate.clone();
    let g = gate.clone();
    let handle = tokio::spawn(async move {
        let mut turn = if with_ctx {
            ApprovalTurnContext::chat(chat_ctx())
        } else {
            ApprovalTurnContext::default()
        };
        if with_ctx && copilot {
            turn = turn.with_copilot_stream();
        }
        turn_origin::with_origin(origin, async move {
            g.intercept(&turn, tool, "characterize", json!({"k": "v"}))
                .await
        })
        .await
    });
    let pending = wait_pending(&gate, tool).await;
    gate.decide(&pending.request_id, ApprovalDecision::ApproveOnce)
        .unwrap();
    assert!(matches!(handle.await.unwrap(), GateOutcome::Allow));
    pending
}

#[tokio::test]
async fn main_chat_park_uses_the_600s_window_and_approve_once_allows() {
    let _lock = env_guard::env_lock_async().await;
    let pending = park_and_approve("char_ttl_default", web_origin(), false, true).await;
    assert_window(&pending, 600);
    assert!(
        pending.source_context.is_none(),
        "chat parks carry no flow context"
    );
    // A decided row leaves the pending list and lands in the audit list.
    assert!(fixture()
        .gate
        .list_pending()
        .unwrap()
        .iter()
        .all(|p| p.request_id != pending.request_id));
    let audit = store::list_recent_decisions(&fixture().config, 100).unwrap();
    let row = audit
        .iter()
        .find(|e| e.request_id == pending.request_id)
        .expect("decided approvals are audited");
    assert_eq!(row.decision, ApprovalDecision::ApproveOnce);
}

#[tokio::test]
async fn copilot_stream_park_is_clamped_to_180s() {
    let _lock = env_guard::env_lock_async().await;
    let pending = park_and_approve("char_ttl_copilot", web_origin(), true, true).await;
    assert_window(&pending, 180);
}

#[tokio::test]
async fn detached_subagent_park_is_clamped_to_180s_and_marked_detached() {
    let _lock = env_guard::env_lock_async().await;
    let fx = fixture();
    // A WebChat origin with no chat context is the async sub-agent path.
    let gate = fx.gate.clone();
    let g = gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            web_origin(),
            g.intercept(
                &ApprovalTurnContext::default(),
                "char_ttl_subagent",
                "child",
                json!({}),
            ),
        )
        .await
    });
    let pending = wait_pending(&gate, "char_ttl_subagent").await;
    assert_window(&pending, 180);
    assert!(gate.request_is_detached(&pending.request_id));
    gate.decide(&pending.request_id, ApprovalDecision::Deny)
        .unwrap();
    assert!(matches!(handle.await.unwrap(), GateOutcome::Deny { .. }));
}

#[tokio::test]
async fn unanswered_park_expires_fail_closed_with_the_unanswered_reason() {
    let _lock = env_guard::env_lock_async().await;
    let fx = fixture();
    // Debug builds honour this override; release builds compile it out.
    std::env::set_var("OPENHUMAN_APPROVAL_TTL_SECS", "1");
    let outcome = turn_origin::with_origin(
        web_origin(),
        fx.gate.intercept(
            &ApprovalTurnContext::chat(chat_ctx()),
            "char_expiry",
            "wait",
            json!({}),
        ),
    )
    .await;
    std::env::remove_var("OPENHUMAN_APPROVAL_TTL_SECS");
    match outcome {
        GateOutcome::Deny { reason } => {
            assert!(is_unanswered_approval_reason(&reason), "{reason}");
            assert!(reason.contains("timed out"), "{reason}");
        }
        other => panic!("an unanswered park must deny, got {other:?}"),
    }
    assert!(
        fx.gate
            .list_pending()
            .unwrap()
            .iter()
            .all(|p| p.tool_name != "char_expiry"),
        "an expired park is no longer actionable"
    );
}

#[tokio::test]
async fn pending_rows_survive_restart_and_expired_rows_fail_closed() {
    let _lock = env_guard::env_lock_async().await;
    let fx = fixture();
    let live = PendingApproval::new(
        "char-restart-live",
        "char_restart",
        "survives",
        json!({}),
        Some(Utc::now() + chrono::Duration::minutes(10)),
    );
    let stale = PendingApproval::new(
        "char-restart-stale",
        "char_restart",
        "expired while down",
        json!({}),
        Some(Utc::now() - chrono::Duration::seconds(5)),
    );
    store::insert_pending(&fx.config, &live, "session-previous-boot").unwrap();
    store::insert_pending(&fx.config, &stale, "session-previous-boot").unwrap();

    // A restart sees only durable state: re-read through a fresh Config handle.
    let restarted = fx.config.clone();
    let listed = store::list_pending(&restarted).unwrap();
    assert!(listed.iter().any(|p| p.request_id == "char-restart-live"));
    assert!(
        listed.iter().all(|p| p.request_id != "char-restart-stale"),
        "rows past their expiry never reappear as actionable"
    );
    assert!(
        store::decide(
            &restarted,
            "char-restart-stale",
            ApprovalDecision::ApproveOnce
        )
        .unwrap()
        .is_none(),
        "deciding an expired row must not approve it"
    );
    let decided = store::decide(&restarted, "char-restart-live", ApprovalDecision::Deny)
        .unwrap()
        .expect("live orphan row can still be decided (DB-only audit update)");
    assert_eq!(decided.request_id, "char-restart-live");
}

#[tokio::test]
async fn cron_and_unknown_origins_deny_external_effects_without_a_row() {
    let _lock = env_guard::env_lock_async().await;
    let fx = fixture();
    let cron = AgentTurnOrigin::TrustedAutomation {
        job_id: "cron-char".into(),
        source: TrustedAutomationSource::Cron,
    };
    let outcome = turn_origin::with_origin(
        cron,
        fx.gate.intercept(
            &ApprovalTurnContext::default(),
            "char_cron",
            "post",
            json!({}),
        ),
    )
    .await;
    assert!(matches!(outcome, GateOutcome::Deny { .. }), "{outcome:?}");
    let outcome = turn_origin::with_origin(
        AgentTurnOrigin::Unknown,
        fx.gate.intercept(
            &ApprovalTurnContext::default(),
            "char_cron",
            "post",
            json!({}),
        ),
    )
    .await;
    assert!(matches!(outcome, GateOutcome::Deny { .. }), "{outcome:?}");
    assert!(fx
        .gate
        .list_pending()
        .unwrap()
        .iter()
        .all(|p| p.tool_name != "char_cron"));

    // Local, user-driven origins pass without a prompt.
    for origin in [AgentTurnOrigin::Cli] {
        let outcome = turn_origin::with_origin(
            origin,
            fx.gate.intercept(
                &ApprovalTurnContext::default(),
                "char_cron",
                "local",
                json!({}),
            ),
        )
        .await;
        assert!(matches!(outcome, GateOutcome::Allow), "{outcome:?}");
    }
}

#[tokio::test]
async fn forced_intercept_fails_closed_without_a_routable_web_origin() {
    let _lock = env_guard::env_lock_async().await;
    let fx = fixture();
    let outcome = turn_origin::with_origin(
        AgentTurnOrigin::Cli,
        fx.gate.intercept_forced("char_forced", "x", json!({})),
    )
    .await;
    assert!(matches!(outcome, GateOutcome::Deny { .. }), "{outcome:?}");
}

async fn bypass_outcome_with_auto_approve_all(tool: &str) -> GateOutcome {
    let fx = fixture();
    let ws = fx.config.workspace_dir.clone();
    let policy = SecurityPolicy {
        auto_approve_all: true,
        ..SecurityPolicy::default()
    };
    live_policy::install(Arc::new(policy), ws.clone(), ws.clone());
    let outcome = turn_origin::with_origin(
        web_origin(),
        fx.gate.intercept(
            &ApprovalTurnContext::chat(chat_ctx()),
            tool,
            "bypass",
            json!({}),
        ),
    )
    .await;
    live_policy::install(Arc::new(SecurityPolicy::default()), ws.clone(), ws);
    outcome
}

#[tokio::test]
async fn auto_approve_all_allows_without_parking_and_writes_no_audit_row_today() {
    let _lock = env_guard::env_lock_async().await;
    let fx = fixture();
    let outcome = bypass_outcome_with_auto_approve_all("char_auto_all").await;
    assert!(matches!(outcome, GateOutcome::Allow), "{outcome:?}");
    assert!(fx
        .gate
        .list_pending()
        .unwrap()
        .iter()
        .all(|p| p.tool_name != "char_auto_all"));
    let audit = store::list_recent_decisions(&fx.config, 500).unwrap();
    assert!(
        audit.iter().all(|e| e.tool_name != "char_auto_all"),
        "the blanket bypass leaves no approval trail today (accepted in #5634)"
    );
    // The unknown-origin exclusion still holds with the bypass on.
    let ws = fx.config.workspace_dir.clone();
    live_policy::install(
        Arc::new(SecurityPolicy {
            auto_approve_all: true,
            ..SecurityPolicy::default()
        }),
        ws.clone(),
        ws.clone(),
    );
    let unknown = turn_origin::with_origin(
        AgentTurnOrigin::Unknown,
        fx.gate.intercept(
            &ApprovalTurnContext::default(),
            "char_auto_all",
            "x",
            json!({}),
        ),
    )
    .await;
    live_policy::install(Arc::new(SecurityPolicy::default()), ws.clone(), ws);
    assert!(matches!(unknown, GateOutcome::Deny { .. }), "{unknown:?}");
}

/// DESIRED: a bypassed call is still auditable. Red until the module-owned gate
/// records an auto-approval row; delete the `should_panic` only when it does.
#[tokio::test]
#[should_panic(expected = "DESIRED-RED")]
async fn desired_auto_approve_all_records_an_audit_row() {
    let _lock = env_guard::env_lock_async().await;
    let fx = fixture();
    let outcome = bypass_outcome_with_auto_approve_all("char_auto_all_audit").await;
    assert!(matches!(outcome, GateOutcome::Allow));
    let audit = store::list_recent_decisions(&fx.config, 500).unwrap();
    assert!(
        audit.iter().any(|e| e.tool_name == "char_auto_all_audit"),
        "DESIRED-RED: auto_approve_all leaves no audit row"
    );
}

#[tokio::test]
async fn flow_trust_is_scoped_to_the_exact_flow_and_tool() {
    let _lock = env_guard::env_lock_async().await;
    let fx = fixture();
    // A trust root (require_approval false) runs without a prompt or a row.
    let outcome = turn_origin::with_origin(
        flow_origin("flow-root", false),
        fx.gate.intercept(
            &ApprovalTurnContext::default(),
            "char_flow_tool",
            "pre-declared",
            json!({}),
        ),
    )
    .await;
    assert!(matches!(outcome, GateOutcome::Allow), "{outcome:?}");

    // require_approval flows park unless the exact (flow, tool) pair is trusted.
    store::insert_flow_trust(&fx.config, "flow-trusted", "char_flow_tool").unwrap();
    store::insert_flow_trust(&fx.config, "flow-trusted", "char_flow_tool").unwrap(); // idempotent
    assert_eq!(
        store::list_flow_trust(&fx.config, "flow-trusted").unwrap(),
        vec!["char_flow_tool".to_string()]
    );
    assert!(store::is_flow_tool_trusted(&fx.config, "flow-trusted", "char_flow_tool").unwrap());
    assert!(!store::is_flow_tool_trusted(&fx.config, "flow-other", "char_flow_tool").unwrap());
    let ctx = || FlowRunContext {
        flow_id: "flow-trusted".into(),
        run_id: "run-char".into(),
    };
    let allowed = turn_origin::with_origin(
        flow_origin("flow-trusted", true),
        fx.gate.intercept(
            &ApprovalTurnContext::default().with_flow_run(ctx()),
            "char_flow_tool",
            "trusted",
            json!({}),
        ),
    )
    .await;
    assert!(matches!(allowed, GateOutcome::Allow), "{allowed:?}");
    assert!(fx
        .gate
        .list_pending()
        .unwrap()
        .iter()
        .all(|p| p.tool_name != "char_flow_tool"));

    // A different tool on the trusted flow still parks (and here is denied).
    let g = fx.gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            flow_origin("flow-trusted", true),
            g.intercept(
                &ApprovalTurnContext::default().with_flow_run(ctx()),
                "char_flow_other",
                "untrusted",
                json!({}),
            ),
        )
        .await
    });
    let pending = wait_pending(&fx.gate, "char_flow_other").await;
    assert!(
        pending.source_context.is_some(),
        "flow parks carry flow/run context"
    );
    fx.gate
        .decide(&pending.request_id, ApprovalDecision::Deny)
        .unwrap();
    assert!(matches!(handle.await.unwrap(), GateOutcome::Deny { .. }));

    // Revocation removes the grant.
    store::delete_flow_trust(&fx.config, "flow-trusted", None).unwrap();
    assert!(!store::is_flow_tool_trusted(&fx.config, "flow-trusted", "char_flow_tool").unwrap());
}

#[tokio::test]
async fn flow_preauthorization_lands_in_the_audit_list_not_pending() {
    let _lock = env_guard::env_lock_async().await;
    let fx = fixture();
    store::record_flow_preauthorization(
        &fx.config,
        "flow-pre",
        "char_pre_tool",
        "session-approval-characterization",
    )
    .unwrap();
    assert!(fx
        .gate
        .list_pending()
        .unwrap()
        .iter()
        .all(|p| p.tool_name != "char_pre_tool"));
    let audit = store::list_recent_decisions(&fx.config, 500).unwrap();
    let row = audit
        .iter()
        .find(|e| e.tool_name == "char_pre_tool")
        .expect("audited");
    assert_eq!(row.decision, ApprovalDecision::ApproveAlwaysForFlow);
}
