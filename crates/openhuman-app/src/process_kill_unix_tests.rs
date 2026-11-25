use super::*;

#[test]
fn signal_pid_refuses_group_and_init_targets() {
    // 0 signals the caller's process group; 1 is init/launchd.
    assert!(signal_pid(0).is_err());
    assert!(signal_pid(1).is_err());
    // Above i32::MAX, `as i32` would go negative: a process-group target.
    assert!(signal_pid(u32::MAX).is_err());
    assert!(signal_pid(i32::MAX as u32 + 1).is_err());
    assert_eq!(signal_pid(2), Ok(2));
    assert_eq!(signal_pid(i32::MAX as u32), Ok(i32::MAX));
}
