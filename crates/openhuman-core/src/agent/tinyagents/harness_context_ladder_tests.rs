use super::*;
use tinyagents_harness::summarization::DEFAULT_SUMMARIZE_KEEP_LAST;

#[test]
fn no_window_and_no_override_installs_no_compression() {
    assert!(compression_policy(None, None).is_none());
    assert!(compression_policy(Some(0), None).is_none());
    assert!(compression_policy(None, Some(0)).is_none());
}

#[test]
fn a_known_window_triggers_at_eighty_percent_capped_at_350k() {
    let policy = compression_policy(Some(200_000), None).expect("policy");
    assert_eq!(policy.trigger_budget(), 160_000);
    assert_eq!(policy.keep_last, DEFAULT_SUMMARIZE_KEEP_LAST);
    // DeepSeek V4 Flash's discovered 1M window compacts at 350k, not ~840k.
    let policy = compression_policy(Some(1_048_576), None).expect("policy");
    assert!(policy.trigger_budget().abs_diff(350_000) <= 1);
}

#[test]
fn the_override_replaces_the_window_fraction() {
    let policy = compression_policy(Some(1_000_000), Some(64_000)).expect("policy");
    assert_eq!(policy.trigger_budget(), 64_000);
    assert!(policy.exceeds_trigger(64_001));
    assert!(!policy.exceeds_trigger(63_000));
    assert_eq!(policy.keep_last, DEFAULT_SUMMARIZE_KEEP_LAST);
}

#[test]
fn the_override_works_without_a_known_window() {
    let policy = compression_policy(None, Some(64_000)).expect("policy");
    assert_eq!(policy.trigger_budget(), 64_000);
    assert_eq!(policy.keep_last, DEFAULT_SUMMARIZE_KEEP_LAST);
}

/// Issue #6960: a compaction that fires mid-turn must keep the turn's
/// assignment verbatim and size its tail in tokens, not by message count.
#[test]
fn the_policy_keeps_a_token_tail_and_pins_the_turn_user_message() {
    let policy = compression_policy(Some(200_000), None).expect("policy");
    assert!(policy.pin_turn_user_message);
    // 30% of the window.
    assert_eq!(policy.keep_recent_tokens, Some(60_000));
}

#[test]
fn the_token_tail_never_exceeds_half_the_trigger() {
    // A 1M window compacts at 350k; 30% of the window (300k) would leave the
    // compaction almost nothing to fold.
    let policy = compression_policy(Some(1_048_576), None).expect("policy");
    assert_eq!(policy.keep_recent_tokens, Some(policy.trigger_budget() / 2));
    let policy = compression_policy(Some(1_000_000), Some(64_000)).expect("policy");
    assert_eq!(policy.keep_recent_tokens, Some(32_000));
}

#[test]
fn an_override_without_a_window_still_pins_with_a_token_tail() {
    let policy = compression_policy(None, Some(64_000)).expect("policy");
    assert!(policy.pin_turn_user_message);
    assert_eq!(policy.keep_recent_tokens, Some(19_200));
}
