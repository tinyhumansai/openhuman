//! Which timeout fired: the command's own `timeout` (exit 124) or the shell
//! tool's `timeout_secs` (#6953).

use super::*;
use crate::tools::implementations::system::shell_support::exec_result;

#[cfg(not(windows))]
fn process_is_running(pid: i32) -> bool {
    if unsafe { libc::kill(pid, 0) } != 0 {
        return false;
    }

    // A killed orphan can remain as a zombie when a container's PID 1 does
    // not reap it. It cannot execute, so treat that kernel state as stopped.
    #[cfg(target_os = "linux")]
    if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        if let Some((_, fields)) = stat.rsplit_once(") ") {
            return !matches!(fields.split_whitespace().next(), Some("Z" | "X" | "x"));
        }
    }

    true
}

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
        out.contains("the shell tool's deadline did not fire"),
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
        out.contains("the shell tool's deadline fired"),
        "tool timeout not attributed: {out}"
    );
}

/// "timed out … and was killed" has to be true of the whole pipeline, not just
/// the shell: the model acts on that message, and a survivor keeps consuming
/// the container the model is still working in.
#[cfg(not(windows))]
#[tokio::test]
async fn the_tools_deadline_kills_the_commands_grandchildren_too() {
    let dir = std::env::temp_dir().join(format!("oh-shell-pgkill-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pidfile = dir.join("grandchild.pid");
    let tool = ShellTool::new(
        test_security(AutonomyLevel::Full),
        test_runtime(),
        test_audit(),
    );
    let result = tool
        .execute(json!({
            "command": format!("sleep 30 & echo $! > {}; wait", pidfile.display()),
            "timeout_secs": 1
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        result.output().contains("timed out after 1s"),
        "{}",
        result.output()
    );

    let grandchild: i32 = std::fs::read_to_string(&pidfile)
        .expect("the shell wrote its background child's pid before the deadline")
        .trim()
        .parse()
        .unwrap();
    let mut gone = false;
    for _ in 0..100 {
        if !process_is_running(grandchild) {
            gone = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        gone,
        "grandchild {grandchild} survived the shell tool's deadline"
    );
}

/// A command killed at its deadline still shows what it printed on both
/// streams: a test that reported its first failures, or a build that wrote
/// its progress to stderr, must not come back as only "timed out".
#[cfg(not(windows))]
#[tokio::test]
async fn the_tools_deadline_keeps_what_the_command_printed() {
    let tool = ShellTool::new(
        test_security(AutonomyLevel::Full),
        test_runtime(),
        test_audit(),
    );
    let result = tool
        .execute(json!({
            "command": "echo 'FAIL: case 3'; echo 'warning: slow' >&2; sleep 5",
            "timeout_secs": 1
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    let out = result.output();
    assert!(
        out.starts_with("Command timed out after 1s"),
        "the timeout line stays first: {out}"
    );
    assert!(out.contains("[stdout]\nFAIL: case 3"), "{out}");
    assert!(out.contains("[stderr]\nwarning: slow"), "{out}");
}

#[test]
fn a_timed_out_result_carries_both_streams_after_the_timeout_line() {
    let result = exec_result(
        Ok(crate::sandbox::SandboxExecResult {
            exit_code: -1,
            stdout: "tested 40 positions\nFAIL: e1d2\n".to_owned(),
            stderr: "warning: slow\n".to_owned(),
            timed_out: true,
        }),
        Duration::from_secs(1331),
    );
    assert!(result.is_error);
    assert_eq!(
        result.output(),
        "Command timed out after 1331s and was killed: the shell tool's deadline fired. \
         Output before the kill:\n[stdout]\ntested 40 positions\nFAIL: e1d2\n[stderr]\nwarning: slow"
    );
}

#[test]
fn a_kill_with_no_output_says_why_output_can_be_missing() {
    let silent = exec_result(
        Ok(crate::sandbox::SandboxExecResult {
            exit_code: -1,
            stdout: String::new(),
            stderr: "\n".to_owned(),
            timed_out: true,
        }),
        Duration::from_secs(5),
    );
    let out = silent.output();
    assert!(
        out.starts_with("Command timed out after 5s and was killed"),
        "{out}"
    );
    assert!(
        out.contains("No output reached the tool before the kill") && out.contains("`| tail`"),
        "{out}"
    );
}
