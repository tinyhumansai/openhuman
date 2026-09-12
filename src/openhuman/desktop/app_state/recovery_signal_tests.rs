use std::sync::Mutex;

use super::*;

/// Both tests below reset and mutate the process-global [`CONFIG_RECOVERED`]
/// atomic. Rust runs unit tests in parallel by default, so without
/// serialization one test's `reset_for_tests()` could clear a value the
/// other just set, between its write and assertion. Hold this guard for the
/// whole body of each test. Recover from poisoning (a panicking test still
/// releases the latch state via the next `reset_for_tests`) so one failure
/// doesn't cascade into spurious lock-poison failures.
static TEST_GUARD: Mutex<()> = Mutex::new(());

#[test]
fn latch_defaults_false_and_marks_true() {
    let _guard = TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    reset_for_tests();
    assert!(
        !config_recovered_this_session(),
        "latch must default to false"
    );
    assert!(
        mark_config_recovered(),
        "first mark reports the false->true transition"
    );
    assert!(
        config_recovered_this_session(),
        "latch must report true after mark"
    );
    // Idempotent — a second mark keeps the latch true but reports no
    // transition, so callers (latch_from_config) log exactly once.
    assert!(
        !mark_config_recovered(),
        "second mark reports no transition (already latched)"
    );
    assert!(config_recovered_this_session());
    reset_for_tests();
}

#[test]
fn latch_from_config_only_marks_when_recovered() {
    let _guard = TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    reset_for_tests();

    let mut clean = Config::default();
    clean.recovered_from_corruption = false;
    latch_from_config(&clean);
    assert!(
        !config_recovered_this_session(),
        "a clean boot config must not latch the signal"
    );

    let mut recovered = Config::default();
    recovered.recovered_from_corruption = true;
    latch_from_config(&recovered);
    assert!(
        config_recovered_this_session(),
        "a recovered boot config must latch the signal"
    );
    reset_for_tests();
}
