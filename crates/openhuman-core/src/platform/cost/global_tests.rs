use super::*;
use crate::platform::cost::tracker_test_lock;
use crate::platform::cost::types::CostRecord;
use tempfile::TempDir;

fn make_usage(input: u64, output: u64, charged: f64) -> BilledUsage {
    BilledUsage::from_counts(input, output).with_charged_usd(charged)
}

#[test]
fn build_token_usage_skips_all_zero_payloads() {
    let usage = make_usage(0, 0, 0.0);
    assert!(build_token_usage("model-a", &usage).is_none());
}

#[test]
fn build_token_usage_populates_fields_and_total() {
    let usage = make_usage(1000, 500, 1.25);
    let translated = build_token_usage("anthropic/claude-sonnet-4", &usage).unwrap();
    assert_eq!(translated.model, "anthropic/claude-sonnet-4");
    assert_eq!(translated.input_tokens, 1000);
    assert_eq!(translated.output_tokens, 500);
    assert_eq!(translated.total_tokens, 1500);
    assert!((translated.cost_usd - 1.25).abs() < f64::EPSILON);
}

#[test]
fn build_token_usage_clamps_nan_and_negative_cost_to_zero() {
    let nan_usage = make_usage(10, 5, f64::NAN);
    let neg_usage = make_usage(10, 5, -3.0);
    let inf_usage = make_usage(10, 5, f64::INFINITY);
    assert_eq!(build_token_usage("m", &nan_usage).unwrap().cost_usd, 0.0);
    assert_eq!(build_token_usage("m", &neg_usage).unwrap().cost_usd, 0.0);
    assert_eq!(build_token_usage("m", &inf_usage).unwrap().cost_usd, 0.0);
}

#[test]
fn build_token_usage_emits_when_tokens_present_even_with_zero_cost() {
    let usage = make_usage(100, 50, 0.0);
    assert!(build_token_usage("m", &usage).is_some());
}

/// Install a global tracker if this process does not already have one, and
/// return the live instance.
///
/// The tracker is a process-wide `OnceCell` shared by every test in this
/// binary — including the runtime-bootstrap tests, which call
/// `platform::cost::init_global` through `core/runtime/bootstrap.rs`. A test therefore
/// cannot assume it owns the global, and **cannot assume the global is
/// absent**: that is precisely why the two tests below assert on records
/// written through whichever tracker is installed, rather than on
/// `try_global()` being `None`. An assertion predicated on emptiness would be
/// order-dependent, and order-dependent is how a guard quietly becomes
/// decorative.
fn global_tracker() -> std::sync::Arc<crate::platform::cost::tracker::CostTracker> {
    let tmp = TempDir::new().unwrap();
    let mut cfg = CostConfig::default();
    cfg.enabled = true;
    init_global(cfg, tmp.path());
    // Keep the directory alive for the call above; records are re-created on
    // write (`CostStorage::add_record` re-makes the path), so a dropped temp
    // dir cannot make the read-backs below flaky.
    std::mem::forget(tmp);
    try_global().expect("init_global must leave a tracker installed")
}

/// Records written by this test only. Every test in this file shares one
/// tracker and one `costs.jsonl`, and libtest runs them in parallel, so each
/// test filters by a model name no other test uses.
fn records_for_model(
    tracker: &crate::platform::cost::tracker::CostTracker,
    model: &str,
) -> Vec<CostRecord> {
    tracker
        .get_recent_records(2, 1000)
        .expect("reading recent cost records must succeed")
        .into_iter()
        .filter(|record| record.usage.model == model)
        .collect()
}

#[test]
fn record_provider_usage_persists_through_the_global_tracker() {
    // Serialised against every other test that touches the process-global
    // tracker; see `tracker_test_lock`.
    let _lock = tracker_test_lock();
    const MODEL: &str = "test-model/record-provider-usage-persists";
    let tracker = global_tracker();

    record_provider_usage(&DispatchCtx::ambient(), MODEL, &make_usage(10, 5, 0.5));

    let records = records_for_model(&tracker, MODEL);
    assert_eq!(
        records.len(),
        1,
        "record_provider_usage must persist exactly one record through the global tracker"
    );
    let usage = &records[0].usage;
    assert_eq!(usage.input_tokens, 10);
    assert_eq!(usage.output_tokens, 5);
    assert_eq!(usage.total_tokens, 15);
    assert!((usage.cost_usd - 0.5).abs() < f64::EPSILON);
}

#[test]
fn record_provider_usage_skips_all_zero_payload() {
    // Serialised against every other test that touches the process-global
    // tracker; see `tracker_test_lock`.
    let _lock = tracker_test_lock();
    const MODEL: &str = "test-model/record-provider-usage-skips-zero";
    let tracker = global_tracker();

    // The all-zero skip lives in `record_provider_usage` itself (the
    // `build_token_usage` `None` arm), so it holds whether or not a tracker is
    // installed — which is what makes this the deterministic half of the old
    // `..._without_global_is_noop` test. That test asserted nothing at all, and
    // its premise ("no GLOBAL_TRACKER initialised in this test process") is not
    // something a shared-process test binary can guarantee.
    record_provider_usage(&DispatchCtx::ambient(), MODEL, &make_usage(0, 0, 0.0));

    assert!(
        records_for_model(&tracker, MODEL).is_empty(),
        "an all-zero usage payload must not reach the tracker"
    );
}

#[test]
fn init_global_is_idempotent() {
    // Serialised against every other test that touches the process-global
    // tracker; see `tracker_test_lock`.
    let _lock = tracker_test_lock();
    // A second `init_global` must be a no-op and must preserve the tracker the
    // first caller installed (`global.rs` early-returns when the cell is set).
    //
    // Asserting that through `try_global()` alone would be vacuous: the slot is
    // only filled when empty, so it refuses the replacement even with the early
    // return deleted, and the *pointer* stays equal either way. The observable that
    // actually distinguishes the two is one level down — `CostTracker::new`
    // eagerly creates `<workspace>/state/` (`CostStorage::new` ->
    // `fs::create_dir_all`). So if the guard is removed, the second call
    // constructs a tracker for the second workspace and that directory appears
    // on disk, even though the instance is then discarded.
    //
    // The test therefore asserts on the filesystem, which is where removing the
    // guard is visible, and on pointer identity, which catches a different
    // regression (swapping the `OnceCell` for storage that can be overwritten).
    let first_dir = TempDir::new().unwrap();
    let second_dir = TempDir::new().unwrap();
    let mut cfg = CostConfig::default();
    cfg.enabled = true;

    // After this the process has a tracker — ours, or one another test in this
    // binary installed first. Either way the *second* call below must no-op, so
    // this assertion holds regardless of test order.
    init_global(cfg.clone(), first_dir.path());
    let before = try_global().expect("init_global must leave a tracker installed");

    let second_state_dir = second_dir.path().join("state");
    assert!(
        !second_state_dir.exists(),
        "second workspace must start clean for this test to mean anything"
    );

    init_global(cfg, second_dir.path());

    assert!(
        !second_state_dir.exists(),
        "second init_global constructed a CostTracker for the second workspace: the \
         idempotence guard in init_global is gone"
    );
    let after = try_global().expect("the tracker must survive a second init_global");
    assert!(
        std::sync::Arc::ptr_eq(&before, &after),
        "second init_global replaced the installed tracker instance"
    );
}

#[test]
fn rebind_global_moves_recording_to_the_new_workspace() {
    let _lock = tracker_test_lock();
    const MODEL_A: &str = "test-model/rebind-before-login";
    const MODEL_B: &str = "test-model/rebind-after-login";
    let previous = try_global();
    let before_login = TempDir::new().unwrap();
    let after_login = TempDir::new().unwrap();
    let mut cfg = CostConfig::default();
    cfg.enabled = true;

    rebind_global(cfg.clone(), before_login.path());
    let first = try_global().expect("rebind installs a tracker");
    record_provider_usage(&DispatchCtx::ambient(), MODEL_A, &make_usage(10, 5, 0.1));

    rebind_global(cfg.clone(), after_login.path());
    let bound = try_global().expect("rebind leaves a tracker installed");
    assert_eq!(bound.workspace_dir(), after_login.path());
    record_provider_usage(&DispatchCtx::ambient(), MODEL_B, &make_usage(20, 10, 0.2));

    // Each workspace keeps its own ledger.
    assert_eq!(records_for_model(&first, MODEL_A).len(), 1);
    assert!(records_for_model(&first, MODEL_B).is_empty());
    assert_eq!(records_for_model(&bound, MODEL_B).len(), 1);
    assert!(records_for_model(&bound, MODEL_A).is_empty());

    rebind_global(cfg, after_login.path());
    let unchanged = try_global().unwrap();
    assert!(
        std::sync::Arc::ptr_eq(&bound, &unchanged),
        "rebinding to the workspace already served must keep the tracker"
    );

    *GLOBAL_TRACKER.write() = previous;
}

/// Persisted cost rows are `TokenUsage`/`CostRecord` JSON lines. Pin the exact
/// serialized key set produced from a provider usage record, and prove a row
/// written by the current release (literal JSON) still deserializes.
#[test]
fn token_usage_row_shape_is_stable_and_old_rows_load() {
    let usage = BilledUsage::from_counts(1000, 500)
        .with_cached_input_tokens(400)
        .with_cache_creation_tokens(30)
        .with_reasoning_tokens(7)
        .with_charged_usd(0.0123);
    let row = build_token_usage("m/x", &usage).unwrap();
    let mut value = serde_json::to_value(&row).unwrap();
    let obj = value.as_object_mut().unwrap();
    obj.remove("timestamp").expect("timestamp key present");
    assert_eq!(
        value,
        serde_json::json!({
            "model": "m/x",
            "input_tokens": 1000,
            "output_tokens": 500,
            "total_tokens": 1500,
            "cached_input_tokens": 400,
            "cache_creation_tokens": 30,
            "reasoning_tokens": 7,
            "cost_usd": 0.0123,
            "cost_source": "provider_charged"
        })
    );

    let old = r#"{"model":"m/x","input_tokens":10,"output_tokens":5,"total_tokens":15,"cost_usd":0.25,"timestamp":"2026-01-02T03:04:05Z"}"#;
    let parsed: TokenUsage = serde_json::from_str(old).expect("old row loads");
    assert_eq!(parsed.input_tokens, 10);
    assert_eq!(parsed.cost_usd, 0.25);
}

#[test]
fn a_tenant_tracker_belongs_to_its_profile_alone() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    let tmp = tempfile::tempdir().unwrap();
    let config = crate::config::Config {
        workspace_dir: tmp.path().to_path_buf(),
        ..crate::config::Config::default()
    };
    let root = CoreContext::for_test(DomainSet::full(), None);
    let derive = |id: &str| {
        root.derive_with(
            ContextOverlay::new(config.clone(), DomainSet::kernel(), Default::default())
                .profile(id),
        )
    };
    let (alice, bob) = (derive("u-alice"), derive("u-bob"));
    assert!(tracker_in(Some(&alice)).is_none());
    seed_tenant_tracker(&alice, &config);
    let tracker = tracker_in(Some(&alice)).expect("seeded at open");
    assert_eq!(tracker.workspace_dir(), tmp.path());
    assert!(tracker_in(Some(&bob)).is_none(), "never another profile's");
    assert!(tracker_in(Some(&root)).is_none(), "nor the operator's");
    assert!(tracker_in(None).is_none(), "an unscoped task has none");
}
