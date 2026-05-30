use super::*;
#[test]
fn reset_local_data_windows_file_lock_error_codes_are_recognized() {
    assert!(is_windows_file_lock_raw_os_error(Some(32)));
    assert!(is_windows_file_lock_raw_os_error(Some(33)));
    assert!(!is_windows_file_lock_raw_os_error(Some(5)));
    assert!(!is_windows_file_lock_raw_os_error(None));
}

#[test]
fn reset_local_data_delete_error_keeps_generic_message_for_other_errors() {
    let err = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
    let result = reset_local_data_delete_error(
        "current openhuman dir",
        std::path::Path::new("/tmp/openhuman"),
        &err,
    );

    let msg = result.expect_err("non-lock errors must still surface to the UI");
    assert!(msg.starts_with("Failed to remove current openhuman dir at /tmp/openhuman:"));
    assert!(!msg.contains("Close all OpenHuman windows and try again"));
}

#[cfg(windows)]
#[test]
fn reset_local_data_delete_error_swallows_lock_failure_when_path_disappeared() {
    // Race condition the reboot fallback now handles: the locked path
    // was gone by the time `schedule_path_for_reboot_deletion` ran its
    // `symlink_metadata` probe, so the reset goal is already met. The
    // helper must return `Ok(())` rather than surfacing a confusing
    // "couldn't remove (it's not there)" toast.
    let dir = tempfile::tempdir().expect("tempdir for reset error test");
    let missing = dir.path().join("definitely-not-there");

    let err = std::io::Error::from_raw_os_error(32);
    let result = reset_local_data_delete_error("current openhuman dir", &missing, &err);

    assert!(
        result.is_ok(),
        "expected NotFound + empty partial schedule to be swallowed as success, got {result:?}"
    );
}

#[cfg(windows)]
#[test]
fn reset_local_data_delete_error_reports_reboot_schedule_counts() {
    // When the lock fallback can walk a real directory tree, the user
    // message should report how much has been queued so the support
    // log preserves "what was actually scheduled". Scheduling itself
    // may still fail at the MoveFileExW step in unprivileged test
    // processes (the registry key write requires administrator); the
    // fallback then carries a partial schedule that the error path
    // surfaces, so both branches must keep mentioning the lock cause
    // *and* expose either the queued counts or the schedule failure.
    let dir = tempfile::tempdir().expect("tempdir for reset error test");
    let target = dir.path().join("reset-mock");
    std::fs::create_dir_all(target.join("nested")).expect("mkdir nested");
    std::fs::write(target.join("a.txt"), b"x").expect("write a.txt");
    std::fs::write(target.join("nested").join("b.txt"), b"y").expect("write b.txt");

    let err = std::io::Error::from_raw_os_error(32);
    let result = reset_local_data_delete_error("current openhuman dir", &target, &err);

    // Path exists on disk, so the fallback must surface the outcome —
    // either an "all-queued" success-but-needs-reboot message (admin)
    // or one of the failure flavours (non-admin).
    let msg =
        result.expect_err("path exists, fallback must report queued counts or scheduling failure");
    let admin_path = msg.contains("queued for deletion the next time you restart Windows")
        && msg.contains("2 files and 2 folders");
    let user_full_fail = msg.contains("scheduling deletion on next reboot also failed");
    let user_partial = msg.contains("queued for the next reboot before scheduling failed");
    assert!(
        admin_path || user_full_fail || user_partial,
        "expected reboot-scheduled, fully-failed, or partial-fail message, got: {msg}"
    );
    // Whatever branch we land on, the user must still be told the lock
    // is what blocked the immediate removal.
    assert!(
        msg.contains("locked by another OpenHuman window or process")
            || msg.contains("another process is holding it open"),
        "missing lock cause: {msg}"
    );
}
