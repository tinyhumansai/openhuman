use super::*;
use std::sync::Mutex;

// The registry is process-global, so these tests would otherwise race:
// one test's `reset_for_tests` can wipe another's entries mid-run. Serialize
// them on a shared lock (poison-tolerant) so each runs against a clean,
// exclusive registry.
static TEST_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn record_returns_true_only_on_first_episode() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_for_tests();
    assert!(record("openrouter", 401), "first record is a new episode");
    assert!(
        !record("openrouter", 401),
        "repeat record of same provider does not re-notify"
    );
    // A different provider is its own episode.
    assert!(record("openai", 403));
    reset_for_tests();
}

#[test]
fn clear_removes_entry_and_rearms_latch() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_for_tests();
    assert!(record("openrouter", 401));
    assert!(clear("openrouter"), "entry was present");
    assert!(!clear("openrouter"), "second clear is a no-op");
    // After clear, the next failure is a new episode again.
    assert!(record("openrouter", 401), "latch re-armed after clear");
    reset_for_tests();
}

#[test]
fn snapshot_is_sorted_and_carries_actionable_message() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_for_tests();
    record("openrouter", 401);
    record("anthropic", 401);
    let snap = snapshot();
    assert_eq!(snap.len(), 2);
    assert_eq!(snap[0].provider, "anthropic");
    assert_eq!(snap[1].provider, "openrouter");
    assert!(snap[1].message.contains("openrouter"));
    assert!(snap[1].message.contains("Connections → API keys → LLM"));
    reset_for_tests();
}
