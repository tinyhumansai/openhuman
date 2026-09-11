//! Steps 1, 2, 3, 4, 5 and 6 at the policy level, without a provider.

use super::*;
use std::sync::Arc;

use crate::openhuman::memory::source_scope::with_source_scope;
use crate::openhuman::security::live_policy;
use crate::openhuman::security::policy::{AutonomyLevel, SecurityPolicy};

use crate::openhuman::memory::guard::test_support::{embedded_policy, external_policy};

/// Install `autonomy` as the live policy for this test thread only.
fn scoped_tier(autonomy: AutonomyLevel) -> live_policy::TestPolicyGuard {
    let dir = std::env::temp_dir();
    live_policy::install_scoped(
        Arc::new(SecurityPolicy {
            autonomy,
            ..SecurityPolicy::default()
        }),
        dir.clone(),
        dir,
    )
}

// ── Step 1 ───────────────────────────────────────────────────────────────────

#[test]
fn guard_with_scoped_full_security_policy_allows() {
    // Other tests may install the process-global policy before this test runs.
    // A thread-local full-tier override keeps the allow-path assertion isolated.
    let _tier = scoped_tier(AutonomyLevel::Full);
    let policy = embedded_policy();
    assert!(policy.enforce_read("core.get").is_ok());
    assert!(policy.enforce_write("core.store").is_ok());
}

#[test]
fn guard_denies_write_under_readonly_tier() {
    let _tier = scoped_tier(AutonomyLevel::ReadOnly);
    let err = embedded_policy()
        .enforce_write("core.store")
        .expect_err("readonly must refuse a write");
    let message = err.to_string();
    assert!(
        message.contains(GUARD_DENIED_PREFIX),
        "refusal must be attributable to the guard: {message}"
    );
    assert!(matches!(err, MemoryError::Invalid(_)));
}

#[test]
fn guard_allows_read_under_readonly_tier() {
    let _tier = scoped_tier(AutonomyLevel::ReadOnly);
    assert!(embedded_policy().enforce_read("core.get").is_ok());
}

#[test]
fn guard_allows_write_under_full_tier() {
    let _tier = scoped_tier(AutonomyLevel::Full);
    assert!(embedded_policy().enforce_write("core.store").is_ok());
}

/// As `scoped_tier`, but with the hourly action budget already exhausted.
fn scoped_tier_with_no_budget(autonomy: AutonomyLevel) -> live_policy::TestPolicyGuard {
    let dir = std::env::temp_dir();
    live_policy::install_scoped(
        Arc::new(SecurityPolicy {
            autonomy,
            max_actions_per_hour: 0,
            ..SecurityPolicy::default()
        }),
        dir.clone(),
        dir,
    )
}

/// The guard's write check is the **tier**, not the agent-tool action budget.
///
/// Fails before the fix: `enforce_write` mapped to `ToolOperation::Act`, whose
/// `record_action` refuses at `max_actions_per_hour = 0` with "Rate limit
/// exceeded". A budget denominated in tool calls must not be charged by a
/// decorator that sits under `MemoryCore::store` — one bulk ingest is hundreds
/// of calls. See `GuardPolicy::enforce_write`.
#[test]
fn guard_write_does_not_consume_the_agent_action_budget() {
    let _tier = scoped_tier_with_no_budget(AutonomyLevel::Full);
    assert!(
        embedded_policy().enforce_write("core.store").is_ok(),
        "an exhausted agent action budget must not refuse a kernel memory write"
    );
}

/// …and the tier refusal the guard *does* own is unaffected by that change.
#[test]
fn guard_still_denies_write_under_readonly_tier_with_budget_exhausted() {
    let _tier = scoped_tier_with_no_budget(AutonomyLevel::ReadOnly);
    assert!(embedded_policy().enforce_write("core.store").is_err());
}

// ── Step 2 ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ambient_scope_is_none_outside_a_source_scope() {
    assert!(embedded_policy().ambient_scope().is_none());
}

#[tokio::test]
async fn ambient_scope_carries_the_task_local_allowlist() {
    with_source_scope(Some(vec!["slack:#eng".into()]), async {
        let scope = embedded_policy().ambient_scope().expect("scoped");
        assert!(scope.allows_source_id("slack:#eng"));
        assert!(scope.allows_source_id("mem_src:slack:#eng:item-1"));
        assert!(!scope.allows_source_id("gmail:me"));
    })
    .await;
}

#[tokio::test]
async fn an_empty_ambient_allowlist_stays_restrictive() {
    // `Some(empty)` must not collapse to `None`: an empty allowlist denies all
    // source-attributed content, per both `source_scope` and `SourceScope`.
    with_source_scope(Some(vec![]), async {
        let scope = embedded_policy().ambient_scope().expect("still restricted");
        assert!(scope.is_empty());
        assert!(!scope.allows_source_id("slack:#eng"));
    })
    .await;
}

#[tokio::test]
async fn narrow_scope_returns_the_ambient_scope_when_the_caller_asks_for_none() {
    with_source_scope(Some(vec!["slack:#eng".into()]), async {
        let scope = embedded_policy().narrow_scope(None).expect("scoped");
        assert_eq!(scope.allow, vec!["slack:#eng".to_string()]);
    })
    .await;
}

#[tokio::test]
async fn narrow_scope_passes_an_explicit_scope_through_when_unrestricted() {
    let requested = SourceScope::new(["gmail:me"]);
    let scope = embedded_policy()
        .narrow_scope(Some(&requested))
        .expect("explicit scope survives");
    assert_eq!(scope.allow, vec!["gmail:me".to_string()]);
}

#[tokio::test]
async fn narrow_scope_keeps_an_explicit_scope_that_is_a_subset_of_the_ambient_one() {
    // The narrowing direction is the legitimate one: a caller that computed a
    // tighter scope than the task-local still wins.
    with_source_scope(
        Some(vec!["slack:#eng".into(), "slack:#ops".into()]),
        async {
            let requested = SourceScope::new(["slack:#eng"]);
            let scope = embedded_policy()
                .narrow_scope(Some(&requested))
                .expect("scoped");
            assert_eq!(scope.allow, vec!["slack:#eng".to_string()]);
        },
    )
    .await;
}

#[tokio::test]
async fn narrow_scope_drops_explicit_sources_outside_the_ambient_allowlist() {
    // The leak this exists to close: an explicit scope must not widen a
    // source-restricted turn back out.
    with_source_scope(Some(vec!["slack:#eng".into()]), async {
        let requested = SourceScope::new(["slack:#eng", "gmail:me"]);
        let scope = embedded_policy()
            .narrow_scope(Some(&requested))
            .expect("scoped");
        assert_eq!(scope.allow, vec!["slack:#eng".to_string()]);
        assert!(!scope.allows_source_id("gmail:me"));
    })
    .await;
}

#[tokio::test]
async fn an_entirely_out_of_scope_request_denies_rather_than_unrestricts() {
    // Empty `Some`, never `None`: an empty allowlist denies all
    // source-attributed content, which is the fail-closed reading.
    with_source_scope(Some(vec!["slack:#eng".into()]), async {
        let requested = SourceScope::new(["gmail:me"]);
        let scope = embedded_policy()
            .narrow_scope(Some(&requested))
            .expect("must stay restricted, not become unrestricted");
        assert!(scope.is_empty());
    })
    .await;
}

// ── Step 3 ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn guard_stamps_internal_taint_outside_a_source_scope() {
    assert_eq!(
        embedded_policy().stamp_taint(MemoryTaint::Internal),
        MemoryTaint::Internal
    );
}

#[tokio::test]
async fn guard_stamps_external_sync_taint_inside_a_source_scope() {
    with_source_scope(Some(vec!["slack:#eng".into()]), async {
        assert_eq!(
            embedded_policy().stamp_taint(MemoryTaint::Internal),
            MemoryTaint::ExternalSync,
            "a source-restricted turn is handling source-attributed content"
        );
    })
    .await;
}

#[tokio::test]
async fn guard_never_downgrades_a_caller_supplied_taint() {
    // The laundering step the contract says the guard exists to prevent: a
    // plain override would rewrite this to `Internal`.
    assert_eq!(
        embedded_policy().stamp_taint(MemoryTaint::ExternalSync),
        MemoryTaint::ExternalSync
    );
}

// ── Step 4 ───────────────────────────────────────────────────────────────────

/// Content that the secret scrubber definitely rewrites, so "no-op" is a real
/// claim rather than an accident of the fixture.
const SECRETY: &str = "Authorization: Bearer abcdefghijklmnop";

#[test]
fn guard_does_not_redact_for_an_embedded_driver() {
    let out = embedded_policy().redact_outbound(SECRETY);
    assert_eq!(out, SECRETY, "embedded traffic must be byte-identical");
    assert!(
        matches!(out, std::borrow::Cow::Borrowed(_)),
        "and must not even be re-allocated"
    );
}

#[test]
fn guard_does_not_redact_for_a_null_driver() {
    let policy = GuardPolicy::new(
        "null",
        DriverClass::Null,
        MemoryHooksConfig::default(),
        TRUSTED,
    );
    assert_eq!(policy.redact_outbound(SECRETY), SECRETY);
}

/// Pins the `Module` pass-through explicitly. The exhaustiveness check on
/// [`GuardPolicy::redact_outbound`]'s `match` guarantees `Module` is handled
/// *somewhere*, but it does not guarantee it stays in the no-op group with
/// `Embedded` — moving it into the `External` arm would silently start
/// sanitizing (corrupting) module-backed memory writes with no compile error
/// to catch it. This test is what would catch that move.
#[test]
fn guard_does_not_redact_for_a_module_driver() {
    let policy = GuardPolicy::new(
        "tinymemory",
        DriverClass::Module,
        MemoryHooksConfig::default(),
        TRUSTED,
    );
    let out = policy.redact_outbound(SECRETY);
    assert_eq!(out, SECRETY, "module traffic must be byte-identical");
    assert!(
        matches!(out, std::borrow::Cow::Borrowed(_)),
        "and must not even be re-allocated"
    );
}

#[test]
fn guard_does_not_redact_json_for_a_module_driver() {
    let policy = GuardPolicy::new(
        "tinymemory",
        DriverClass::Module,
        MemoryHooksConfig::default(),
        TRUSTED,
    );
    let value = serde_json::json!({ "token": SECRETY });
    assert_eq!(policy.redact_outbound_json(value.clone()), value);
}

#[test]
fn guard_redacts_content_for_an_external_driver() {
    let out = external_policy(TRUSTED).redact_outbound(SECRETY);
    assert_ne!(out, SECRETY);
    assert!(
        out.contains("[REDACTED]"),
        "expected the crate scrubber's placeholder, got: {out}"
    );
}

#[test]
fn guard_redacts_json_only_for_an_external_driver() {
    let value = serde_json::json!({ "token": SECRETY });
    assert_eq!(
        embedded_policy().redact_outbound_json(value.clone()),
        value,
        "embedded JSON is untouched"
    );
    assert_ne!(
        external_policy(TRUSTED).redact_outbound_json(value.clone()),
        value
    );
}

// ── Step 5 ───────────────────────────────────────────────────────────────────

#[test]
fn guard_never_gates_egress_for_an_embedded_driver() {
    assert!(embedded_policy().check_egress("core.store", true).is_ok());
}

#[test]
fn guard_refuses_an_external_driver_with_untrusted_state() {
    let err = external_policy("untrusted")
        .check_egress("core.store", true)
        .expect_err("fail-closed");
    let message = err.to_string();
    assert!(message.contains(GUARD_DENIED_PREFIX), "{message}");
    assert!(message.contains("untrusted"), "{message}");
}

#[test]
fn guard_admits_an_external_driver_whose_trust_was_raised() {
    assert!(external_policy(TRUSTED)
        .check_egress("core.store", true)
        .is_ok());
}

// ── Step 6 ───────────────────────────────────────────────────────────────────

#[test]
fn budgets_come_from_the_hooks_config() {
    let policy = GuardPolicy::new(
        "tinycortex",
        DriverClass::Embedded,
        MemoryHooksConfig {
            recall_max_chars: 42,
            capture_max_chars: 7,
            ..MemoryHooksConfig::default()
        },
        TRUSTED,
    );
    assert_eq!(policy.recall_budget(), Some(42));
    assert_eq!(policy.capture_budget(), Some(7));
}

#[test]
fn a_zero_budget_reads_as_disabled_not_as_deny_everything() {
    let policy = GuardPolicy::new(
        "tinycortex",
        DriverClass::Embedded,
        MemoryHooksConfig {
            recall_max_chars: 0,
            capture_max_chars: 0,
            ..MemoryHooksConfig::default()
        },
        TRUSTED,
    );
    assert_eq!(policy.recall_budget(), None);
    assert_eq!(policy.capture_budget(), None);
}
