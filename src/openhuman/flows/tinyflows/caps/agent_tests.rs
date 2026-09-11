// --- harness fan-out concurrency ceiling ---

#[test]
fn harness_ceiling_defaults_when_unset_or_nonsense() {
    // A malformed override must never produce a zero-permit semaphore —
    // that would deadlock every flow agent node in the process.
    for raw in [None, Some(""), Some("0"), Some("-4"), Some("lots")] {
        assert_eq!(
            super::max_parallel_harness_agents(raw),
            super::DEFAULT_MAX_PARALLEL_HARNESS_AGENTS,
            "{raw:?} should fall back to the default"
        );
    }
}

#[test]
fn harness_ceiling_honours_a_valid_override() {
    assert_eq!(super::max_parallel_harness_agents(Some("3")), 3);
    assert_eq!(super::max_parallel_harness_agents(Some(" 16 ")), 16);
}

#[test]
fn explicit_timeout_is_clamped_but_never_scaled() {
    assert_eq!(super::resolve_run_timeout_secs(Some(120), 50), 120);
    assert_eq!(super::resolve_run_timeout_secs(Some(5), 50), 10);
    assert_eq!(super::resolve_run_timeout_secs(Some(9_000), 50), 600);
}

#[test]
fn default_timeout_scales_with_iteration_cap_and_caps_at_600() {
    assert_eq!(super::resolve_run_timeout_secs(None, 10), 240);
    assert_eq!(super::resolve_run_timeout_secs(None, 25), 300);
    assert_eq!(super::resolve_run_timeout_secs(None, 50), 600);
    assert_eq!(super::resolve_run_timeout_secs(None, usize::MAX), 600);
}

#[tokio::test]
async fn production_harness_ceiling_is_open_and_reusable() {
    let held = super::HARNESS_AGENT_SLOTS
        .acquire()
        .await
        .expect("the production limiter must remain open");
    drop(held);
    assert!(!super::HARNESS_AGENT_SLOTS.is_closed());
}
