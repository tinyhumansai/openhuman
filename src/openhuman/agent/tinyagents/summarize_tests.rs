use super::*;

#[test]
fn policy_is_context_window_aware_at_the_configured_threshold() {
    let policy = summarization_policy(200_000);
    assert_eq!(policy.context_window, Some(200_000));
    assert_eq!(policy.threshold_fraction, SUMMARIZE_THRESHOLD_FRACTION);
    assert_eq!(policy.keep_last, SUMMARIZE_KEEP_LAST);
}

#[test]
fn threshold_fraction_leaves_headroom_below_the_window() {
    // The policy must trigger *before* the window is full, so the summary
    // call itself has room — 90% by default.
    assert!(SUMMARIZE_THRESHOLD_FRACTION > 0.0 && SUMMARIZE_THRESHOLD_FRACTION < 1.0);
    let policy = summarization_policy(100_000);
    // 90% of 100k = 90k tokens is the effective trigger point.
    let effective = (policy.context_window.unwrap() as f64 * policy.threshold_fraction) as u64;
    assert_eq!(effective, 90_000);
}
