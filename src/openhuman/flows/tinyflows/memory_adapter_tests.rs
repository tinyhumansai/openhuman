use super::*;
use crate::openhuman::security::{AutonomyLevel, POLICY_BLOCKED_MARKER};
use tempfile::TempDir;

fn security(autonomy: AutonomyLevel) -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy {
        autonomy,
        ..SecurityPolicy::default()
    })
}

/// A `Config` rooted at a fresh tempdir — `lookup_flavour` (via
/// `flavour`) touches disk under `workspace_dir`, so every test needs its
/// own isolated root rather than sharing whatever `Config::default()`'s
/// workspace_dir happens to resolve to. The `TempDir` guard must outlive
/// the adapter, so callers keep it alive for the test's duration.
fn test_config() -> (TempDir, Arc<Config>) {
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    (tmp, Arc::new(cfg))
}

fn adapter(autonomy: AutonomyLevel) -> (TempDir, OpenHumanMemory) {
    let (tmp, config) = test_config();
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
/// (the tier-gate write summary is fine, but `flow_memory_namespace` —
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

// ── flavour / people delegate cleanly with no trusted origin required ──

#[tokio::test]
async fn flavour_unknown_slug_errs() {
    let (_tmp, adapter) = adapter(AutonomyLevel::Full);
    let err = adapter.flavour("not-a-real-flavour").await.unwrap_err();
    assert!(err.to_string().contains("Unknown flavour"));
}

#[tokio::test]
async fn flavour_valid_slug_with_no_tree_yet_reports_not_found() {
    let (_tmp, adapter) = adapter(AutonomyLevel::Full);
    let result = adapter.flavour("coding_style").await.unwrap();
    assert_eq!(result["found"], json!(false));
    assert_eq!(result["profile"], Value::Null);
}

#[tokio::test]
async fn flavour_blocked_in_readonly_autonomy_still_reaches_lookup() {
    // Read is Allow at every tier, so ReadOnly must behave identically
    // to Full for a read-only operation like `flavour`.
    let (_tmp, adapter) = adapter(AutonomyLevel::ReadOnly);
    let result = adapter.flavour("coding_style").await.unwrap();
    assert_eq!(result["found"], json!(false));
}

// ── value_to_content / filter_people_by_query (pure helpers) ──────────

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
fn filter_people_by_query_matches_display_name_case_insensitively() {
    let listing = json!({
        "people": [
            {"display_name": "Ada Lovelace", "primary_email": null, "primary_phone": null, "handles": []},
            {"display_name": "Grace Hopper", "primary_email": null, "primary_phone": null, "handles": []},
        ]
    });
    let filtered = filter_people_by_query(listing, "ada");
    let people = filtered["people"].as_array().unwrap();
    assert_eq!(people.len(), 1);
    assert_eq!(people[0]["display_name"], json!("Ada Lovelace"));
}

#[test]
fn filter_people_by_query_matches_handle_values() {
    let listing = json!({
        "people": [
            {
                "display_name": "Someone",
                "primary_email": null,
                "primary_phone": null,
                "handles": [{"kind": "email", "value": "someone@example.com"}]
            },
        ]
    });
    let filtered = filter_people_by_query(listing, "example.com");
    assert_eq!(filtered["people"].as_array().unwrap().len(), 1);
}

#[test]
fn filter_people_by_query_no_match_returns_empty() {
    let listing = json!({
        "people": [
            {"display_name": "Ada Lovelace", "primary_email": null, "primary_phone": null, "handles": []},
        ]
    });
    let filtered = filter_people_by_query(listing, "zzz-no-match");
    assert!(filtered["people"].as_array().unwrap().is_empty());
}
