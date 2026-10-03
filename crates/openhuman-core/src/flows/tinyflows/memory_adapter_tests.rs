use super::*;
use crate::security::{AutonomyLevel, POLICY_BLOCKED_MARKER};
use tempfile::TempDir;

fn security(autonomy: AutonomyLevel) -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy {
        autonomy,
        ..SecurityPolicy::default()
    })
}

/// A `Config` rooted at a fresh tempdir with an in-memory reference engine
/// bound to it. The `TempDir` guard must outlive the adapter, so callers keep
/// it alive for the test's duration.
fn test_config() -> (TempDir, Arc<Config>) {
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    (tmp, Arc::new(cfg))
}

fn adapter(autonomy: AutonomyLevel) -> (TempDir, OpenHumanMemory) {
    let (tmp, config) = test_config();
    crate::memory::engine::install_test_engine(
        &config.workspace_dir,
        Arc::new(tinymemory::conformance::ReferenceEngine::new()),
    );
    (
        tmp,
        OpenHumanMemory {
            config,
            security: security(autonomy),
        },
    )
}

// ── scope lockdown (defense-in-depth) ────────────────────────────────

#[tokio::test]
async fn remember_rejects_user_scope() {
    let (_tmp, adapter) = adapter(AutonomyLevel::Full);
    let err = adapter.remember("user", "k", json!("v")).await.unwrap_err();
    assert!(err.to_string().contains("only supports scope \"flow\""));
}

#[tokio::test]
async fn remember_rejects_flows_scope() {
    let (_tmp, adapter) = adapter(AutonomyLevel::Full);
    let err = adapter
        .remember("flows", "k", json!("v"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("only supports scope \"flow\""));
}

#[tokio::test]
async fn forget_rejects_user_scope() {
    let (_tmp, adapter) = adapter(AutonomyLevel::Full);
    let err = adapter.forget("user", "k").await.unwrap_err();
    assert!(err.to_string().contains("only supports scope \"flow\""));
}

#[tokio::test]
async fn forget_rejects_flows_scope() {
    let (_tmp, adapter) = adapter(AutonomyLevel::Full);
    let err = adapter.forget("flows", "k").await.unwrap_err();
    assert!(err.to_string().contains("only supports scope \"flow\""));
}

#[tokio::test]
async fn remember_rejects_empty_key_even_at_flow_scope() {
    let (_tmp, adapter) = adapter(AutonomyLevel::Full);
    let err = adapter
        .remember("flow", "   ", json!("v"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("non-empty key"));
}

// ── recall scope validation ───────────────────────────────────────────

#[tokio::test]
async fn recall_rejects_unknown_scope() {
    let (_tmp, adapter) = adapter(AutonomyLevel::Full);
    let err = adapter
        .recall("nonsense", "q", json!({"operation": "recall"}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("unknown scope"));
}

// ── flow-id trust boundary ─────────────────────────────────────────────

#[tokio::test]
async fn remember_flow_scope_without_trusted_origin_errs() {
    // No `turn_origin::current()` scoped — not running inside a flow.
    let (_tmp, adapter) = adapter(AutonomyLevel::Full);
    let err = adapter.remember("flow", "k", json!("v")).await.unwrap_err();
    assert!(err.to_string().contains("trusted Workflow-scoped origin"));
}

// ── secret check runs BEFORE the HITL approval gate (P1 review fix) ──

/// Regression for the review finding that `has_likely_secret` ran AFTER
/// `tier_gate_write` (which can park the run for human approval via
/// `gate_call_for_tier`) — a user could approve a write that then silently
/// failed the secret heuristic, wasting the approval round-trip.
///
/// This is asserted indirectly but unambiguously: with NO trusted
/// `TrustedAutomation { Workflow }` origin scoped, `remember`'s later steps
/// (the tier-gate write summary is fine, but the trusted flow id —
/// reached only AFTER the tier gate — requires one and errors
/// `"trusted Workflow-scoped origin"` if missing, see
/// `remember_flow_scope_without_trusted_origin_errs` above). If the secret
/// check now runs strictly first, a secret-shaped value must fail with the
/// secret-rejection message instead of ever reaching that later trusted-origin
/// check — proving the reject happens before both the approval gate AND the
/// flow-id resolution that follows it.
#[tokio::test]
async fn remember_rejects_secret_before_reaching_trusted_origin_or_approval_gate() {
    let (_tmp, adapter) = adapter(AutonomyLevel::Full);
    let err = adapter
        .remember("flow", "k", json!("api_key=abc123"))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("looks like a secret"),
        "expected the secret rejection to fire first, got: {err}"
    );
    assert!(
        !err.to_string().contains("trusted Workflow-scoped origin"),
        "secret check must short-circuit before flow-id/approval resolution, got: {err}"
    );
}

/// Same proof under `Supervised` autonomy, where `CommandClass::Write` is
/// `GateDecision::Prompt` and — with a real `ApprovalGate` installed —
/// `tier_gate_write` would park for human approval. The secret rejection
/// must still win the race: it never even calls into the tier gate.
#[tokio::test]
async fn remember_rejects_secret_under_supervised_autonomy_without_approval_round_trip() {
    let (_tmp, adapter) = adapter(AutonomyLevel::Supervised);
    let err = adapter
        .remember("flow", "k", json!("Bearer abcdefghijklmnopqrstuvwxyz"))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("looks like a secret"),
        "expected the secret rejection to fire before any approval prompt, got: {err}"
    );
}

#[tokio::test]
async fn recall_flow_scope_without_trusted_origin_errs() {
    let (_tmp, adapter) = adapter(AutonomyLevel::Full);
    let err = adapter
        .recall("flow", "q", json!({"operation": "recall"}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("trusted Workflow-scoped origin"));
}

// ── tier gate ───────────────────────────────────────────────────────────

#[tokio::test]
async fn recall_blocked_in_readonly_autonomy_never_touches_flow_id() {
    // ReadOnly + CommandClass::Read is still Allow (Read is always
    // allowed) — so this proves the tier gate runs and lets a genuine
    // read through even under the most restrictive tier, distinct from
    // the write path below.
    let (_tmp, adapter) = adapter(AutonomyLevel::ReadOnly);
    // No trusted origin, so this will fail on the flow-id lookup, not on
    // the tier gate — proving the tier gate (Read => Allow) did not
    // block it first.
    let err = adapter
        .recall("flow", "q", json!({"operation": "recall"}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("trusted Workflow-scoped origin"));
}

#[tokio::test]
async fn remember_blocked_in_readonly_autonomy() {
    let (_tmp, adapter) = adapter(AutonomyLevel::ReadOnly);
    let err = adapter.remember("flow", "k", json!("v")).await.unwrap_err();
    assert!(err.to_string().contains(POLICY_BLOCKED_MARKER));
}

#[tokio::test]
async fn forget_blocked_in_readonly_autonomy() {
    let (_tmp, adapter) = adapter(AutonomyLevel::ReadOnly);
    let err = adapter.forget("flow", "k").await.unwrap_err();
    assert!(err.to_string().contains(POLICY_BLOCKED_MARKER));
}

// ── flavour / people: unsupported in memory v2 ──────────────────────────

#[tokio::test]
async fn flavour_reports_every_slug_unknown() {
    let (_tmp, adapter) = adapter(AutonomyLevel::Full);
    let err = adapter.flavour("coding_style").await.unwrap_err();
    assert!(err.to_string().contains("unknown flavour"));
}

#[tokio::test]
async fn people_returns_an_empty_unsupported_listing() {
    let (_tmp, adapter) = adapter(AutonomyLevel::ReadOnly);
    let result = adapter.people(Some("ada")).await.unwrap();
    assert_eq!(result["people"], json!([]));
    assert_eq!(result["supported"], json!(false));
}

// ── round trip inside a trusted workflow run ──────────────────────────

fn workflow_origin(flow_id: &str) -> AgentTurnOrigin {
    AgentTurnOrigin::TrustedAutomation {
        job_id: flow_id.to_string(),
        source: TrustedAutomationSource::Workflow {
            require_approval: false,
        },
    }
}

#[tokio::test]
async fn remember_search_and_forget_round_trip_in_the_flows_own_scope() {
    let (_tmp, adapter) = adapter(AutonomyLevel::Full);
    turn_origin::with_origin(workflow_origin("f1"), async {
        adapter
            .remember("flow", "sent", json!("newsletter item 42"))
            .await
            .unwrap();
        adapter
            .remember("flow", "sent", json!("newsletter item 43"))
            .await
            .unwrap();
        let found = adapter
            .recall("flow", "newsletter item", json!({"operation": "search"}))
            .await
            .unwrap();
        let results = found["results"].as_array().unwrap();
        assert_eq!(results.len(), 1, "a key rewrite replaces the old value");
        assert_eq!(results[0]["key"], json!("sent"));
        let text = results[0]["text"].as_str().unwrap();
        assert!(text.contains("item 43"));
        assert!(
            text.contains("untrusted-source"),
            "flow output is marked as data"
        );

        let recalled = adapter
            .recall("flows", "newsletter item", json!({"operation": "recall"}))
            .await
            .unwrap();
        assert!(recalled["answer"].is_string());
        assert_eq!(recalled["results"].as_array().unwrap().len(), 1);

        adapter.forget("flow", "sent").await.unwrap();
        adapter.forget("flow", "never-written").await.unwrap();
        let after = adapter
            .recall("flow", "newsletter item", json!({"operation": "search"}))
            .await
            .unwrap();
        assert!(after["results"].as_array().unwrap().is_empty());
    })
    .await;
}

#[tokio::test]
async fn flow_scope_never_sees_another_flows_items() {
    let (_tmp, adapter) = adapter(AutonomyLevel::Full);
    turn_origin::with_origin(workflow_origin("f-other"), async {
        adapter
            .remember("flow", "k", json!("other flow note"))
            .await
            .unwrap();
    })
    .await;
    turn_origin::with_origin(workflow_origin("f-mine"), async {
        let found = adapter
            .recall("flow", "other flow note", json!({"operation": "search"}))
            .await
            .unwrap();
        assert!(found["results"].as_array().unwrap().is_empty());
    })
    .await;
}

// ── pure helpers ───────────────────────────────────────────────────────

#[test]
fn value_to_content_keeps_strings_verbatim() {
    assert_eq!(
        value_to_content(&json!("already published")),
        "already published"
    );
}

#[test]
fn value_to_content_serializes_non_strings() {
    assert_eq!(value_to_content(&json!({"id": 42})), "{\"id\":42}");
    assert_eq!(value_to_content(&json!(42)), "42");
    assert_eq!(value_to_content(&json!(true)), "true");
}

#[test]
fn min_score_keeps_unscored_rows() {
    assert!(passes_min_score(None, Some(0.9)));
    assert!(passes_min_score(Some(0.5), None));
    assert!(!passes_min_score(Some(0.1), Some(0.5)));
}

#[test]
fn plain_agent_learnings_are_trusted_and_everything_else_is_not() {
    assert!(!is_untrusted(&MemoryMeta::default()));
    assert!(is_untrusted(&crate::flows::flow_meta("f", &[])));
    assert!(is_untrusted(&MemoryMeta::from_source(
        SourceKind::Composio,
        None
    )));
}
