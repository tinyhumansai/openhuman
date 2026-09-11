use super::*;

#[test]
fn noop_sandbox_name() {
    assert_eq!(NoopSandbox.name(), "none");
}

#[test]
fn noop_sandbox_is_always_available() {
    assert!(NoopSandbox.is_available());
}

#[test]
fn noop_sandbox_wrap_command_is_noop() {
    let mut cmd = Command::new("echo");
    cmd.arg("test");
    let original_program = cmd.get_program().to_string_lossy().to_string();
    let original_args: Vec<String> = cmd
        .get_args()
        .map(|s| s.to_string_lossy().to_string())
        .collect();

    let sandbox = NoopSandbox;
    assert!(sandbox.wrap_command(&mut cmd).is_ok());

    // Command should be unchanged
    assert_eq!(cmd.get_program().to_string_lossy(), original_program);
    assert_eq!(
        cmd.get_args()
            .map(|s| s.to_string_lossy().to_string())
            .collect::<Vec<_>>(),
        original_args
    );
}
