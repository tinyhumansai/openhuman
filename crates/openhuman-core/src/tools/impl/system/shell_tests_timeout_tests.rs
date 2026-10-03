//! Which timeout fired: the command's own `timeout` (exit 124) or the shell
//! tool's `timeout_secs` (#6953).

use super::*;

/// Exit 124 is what coreutils `timeout` returns when its limit expires. The
/// tool's own deadline did not fire, and the model must be told which one did
/// — otherwise `Command failed (exit code 124) [stdout] ok` reads like a bug.
#[cfg(not(windows))]
#[tokio::test]
async fn exit_124_is_attributed_to_the_commands_own_timeout() {
    let tool = ShellTool::new(
        test_security(AutonomyLevel::Full),
        test_runtime(),
        test_audit(),
    );
    let result = tool
        .execute(json!({"command": "echo ok; exit 124"}))
        .await
        .unwrap();
    assert!(result.is_error);
    let out = result.output();
    assert!(out.contains("exit code 124"), "{out}");
    assert!(
        out.contains("the command's own `timeout` limit expired"),
        "exit 124 not attributed: {out}"
    );
    assert!(
        out.contains("the shell tool's timeout_secs did not fire"),
        "{out}"
    );
}

#[cfg(not(windows))]
#[tokio::test]
async fn other_failures_carry_no_timeout_attribution() {
    let tool = ShellTool::new(
        test_security(AutonomyLevel::Full),
        test_runtime(),
        test_audit(),
    );
    let result = tool.execute(json!({"command": "exit 3"})).await.unwrap();
    assert!(!result.output().contains("timeout"), "{}", result.output());
}

#[cfg(not(windows))]
#[tokio::test]
async fn the_tools_own_deadline_says_it_was_timeout_secs() {
    let tool = ShellTool::new(
        test_security(AutonomyLevel::Full),
        test_runtime(),
        test_audit(),
    );
    let result = tool
        .execute(json!({"command": "sleep 5", "timeout_secs": 1}))
        .await
        .unwrap();
    assert!(result.is_error);
    let out = result.output();
    assert!(out.contains("timed out after 1s"), "{out}");
    assert!(
        out.contains("the shell tool's timeout_secs limit fired"),
        "tool timeout not attributed: {out}"
    );
}
