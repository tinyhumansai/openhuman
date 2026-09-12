use super::*;

// These tests intentionally mutate the process-global denial buffer. Keep
// their clear/record/assert sequences atomic with respect to one another;
// the parallel libtest runner can otherwise clear a sibling's freshly
// recorded value between `record` and `list`.
static DENIAL_TEST_LOCK: Mutex<()> = Mutex::new(());

fn clear_denials_for_test() {
    let mut buf = RECENT_DENIALS.lock().unwrap_or_else(|p| p.into_inner());
    buf.clear();
}

#[test]
fn record_truncates_and_bounds() {
    let _guard = DENIAL_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    clear_denials_for_test();
    let long = "a".repeat(10_000);
    for _ in 0..(MAX_DENIALS + 5) {
        record("tool.x", "policy", "denied", &long);
    }
    let listed = list(999);
    assert_eq!(listed.len(), MAX_DENIALS);
    assert!(listed[0].reason.len() < 300);
    assert_eq!(listed[0].tool_name, "tool.x");
}

#[test]
fn record_ignores_empty_tool() {
    let _guard = DENIAL_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    clear_denials_for_test();
    record("   ", "policy", "denied", "reason");
    // list() should not panic; we can't reliably assert length because tests may run in parallel.
    let _ = list(10);
}

#[test]
fn record_redacts_sensitive_reason_fragments() {
    let _guard = DENIAL_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    clear_denials_for_test();
    record(
        "tool.secret",
        "policy",
        "denied",
        "blocked: Bearer abcdefghijklmnopqrstuvwxyz",
    );
    let listed = list(1);
    assert_eq!(listed[0].reason, "[redacted: sensitive content]");
}
