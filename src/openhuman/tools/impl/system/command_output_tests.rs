use super::*;

#[test]
fn failure_surfaces_exit_code_and_both_streams() {
    let rendered = render_command_failure(Some(7), "the-stdout-line", "the-stderr-line");
    assert!(
        rendered.contains("exit code 7"),
        "exit code missing: {rendered}"
    );
    assert!(
        rendered.contains("the-stdout-line"),
        "stdout dropped on failure: {rendered}"
    );
    assert!(
        rendered.contains("the-stderr-line"),
        "stderr dropped on failure: {rendered}"
    );
}

#[test]
fn failure_keeps_stdout_even_when_stderr_present() {
    // The exact regression: the old `if stderr.is_empty() { stdout } else
    // { stderr }` formatting threw stdout away whenever stderr existed.
    let rendered = render_command_failure(Some(1), "diagnostic-on-stdout", "error-on-stderr");
    assert!(rendered.contains("diagnostic-on-stdout"));
    assert!(rendered.contains("error-on-stderr"));
}

#[test]
fn exit_127_hints_missing_command_or_dependency() {
    let rendered = render_command_failure(Some(127), "", "pytest: command not found");
    assert!(rendered.contains("exit code 127"));
    assert!(
        rendered.to_lowercase().contains("command not found"),
        "127 should hint at a missing command/dependency: {rendered}"
    );
}

#[test]
fn exit_126_hints_permission_or_sandbox() {
    let rendered = render_command_failure(Some(126), "", "permission denied");
    assert!(rendered.contains("exit code 126"));
    assert!(
        rendered.to_lowercase().contains("sandbox")
            || rendered.to_lowercase().contains("permission denied"),
        "126 should hint at a permission/sandbox wall: {rendered}"
    );
}

#[test]
fn ordinary_failure_code_gets_no_hint() {
    let rendered = render_command_failure(Some(1), "", "boom");
    // No editorialising for a generic application failure.
    assert!(rendered.contains("exit code 1"));
    assert!(!rendered.contains("command not found"));
    assert!(!rendered.contains("sandbox"));
}

#[test]
fn signal_termination_has_no_exit_code() {
    let rendered = render_command_failure(None, "", "");
    assert!(rendered.contains("terminated by a signal"));
    assert!(rendered.contains("no output was captured"));
}

#[test]
fn sandbox_negative_exit_code_maps_to_signal() {
    assert_eq!(sandbox_exit_code(-1), None);
    assert_eq!(sandbox_exit_code(0), Some(0));
    assert_eq!(sandbox_exit_code(7), Some(7));
}
