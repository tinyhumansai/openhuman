use super::*;

/// `test_host::absent()` pins the path a process that never armed TinySecurity
/// takes. The module config is process-wide, so once any other test in the
/// run has armed it, the guard has to hide it on this thread too, or the
/// "unarmed" test silently runs against an armed classifier.
#[test]
fn an_absent_thread_does_not_see_a_config_armed_by_another_test() {
    let armed = Some(Config::default());
    assert!(visible(armed.clone()).is_some(), "armed is visible by default");
    let _absent = super::super::test_host::absent();
    assert!(
        visible(armed).is_none(),
        "absent() must hide the process-wide armed config"
    );
}
