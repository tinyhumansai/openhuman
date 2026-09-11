use super::*;

#[cfg(unix)]
#[test]
fn command_output_with_timeout_returns_output_for_fast_command() {
    let mut command = Command::new("sh");
    command.arg("-c").arg("printf ready");

    let output =
        command_output_with_timeout("test fast command", &mut command, Duration::from_secs(1))
            .expect("fast command should complete");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "ready");
}

#[cfg(unix)]
#[test]
fn command_output_with_timeout_kills_slow_command() {
    let mut command = Command::new("sh");
    command.arg("-c").arg("sleep 2; printf late");

    let error =
        command_output_with_timeout("test slow command", &mut command, Duration::from_millis(50))
            .expect_err("slow command should time out");

    assert!(error.contains("timed out after"));
}
