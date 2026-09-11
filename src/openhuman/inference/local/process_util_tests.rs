use super::*;

#[test]
fn apply_no_window_is_callable_on_every_platform() {
    // The function is a no-op on non-Windows. On Windows it sets a
    // creation flag we cannot directly read back from
    // `tokio::process::Command`, so this test just guarantees the
    // helper compiles and is callable from generic code.
    let mut cmd = tokio::process::Command::new("does-not-need-to-exist");
    apply_no_window(&mut cmd);
}
