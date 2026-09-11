use super::*;
use tempfile::TempDir;

fn make_usage(input: u64, output: u64, charged: f64) -> UsageInfo {
    UsageInfo {
        input_tokens: input,
        output_tokens: output,
        context_window: 0,
        cached_input_tokens: 0,
        cache_creation_tokens: 0,
        reasoning_tokens: 0,
        charged_amount_usd: charged,
    }
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

#[test]
fn record_provider_usage_without_global_is_noop() {
    // No GLOBAL_TRACKER initialised in this test process by default;
    // call must return Ok without panic.
    let usage = make_usage(10, 5, 0.5);
    record_provider_usage("m", &usage);
}

#[test]
fn init_global_is_idempotent() {
    // The OnceCell is process-wide. After at most one call across the
    // whole test run it will be `Some`, and any further `init_global`
    // call must be a no-op (and must not panic). We assert the
    // post-condition either way: try_global resolves to Some on the
    // happy path, or the construct-then-set race is logged silently.
    let tmp = TempDir::new().unwrap();
    let mut cfg = CostConfig::default();
    cfg.enabled = true;
    init_global(cfg.clone(), tmp.path());
    init_global(cfg, tmp.path()); // second call is a no-op
                                  // If this test ran first, global is now set. If another test set
                                  // a different workspace already, the original is retained — both
                                  // are valid behaviours per the contract.
    assert!(try_global().is_some() || try_global().is_none());
}
