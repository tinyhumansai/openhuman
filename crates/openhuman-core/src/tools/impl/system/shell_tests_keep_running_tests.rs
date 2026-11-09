//! A shell command past its yield point keeps running, and `shell_process`
//! follows it, within one turn.

use super::*;
use crate::agent::tinyagents::host::run_context::test_turn_run;
use crate::tools::implementations::system::ShellProcessTool;
use tinyagents_harness::ids::CallId;
use tinyagents_harness::tool::ToolExecutionContext;

fn shell() -> ShellTool {
    ShellTool::new(
        test_security(AutonomyLevel::Full),
        test_runtime(),
        test_audit(),
    )
}

#[cfg(not(windows))]
#[tokio::test]
async fn a_long_command_yields_and_shell_process_follows_it_to_its_end() {
    let run = test_turn_run();
    let first = ToolExecutionContext::from_run_context(&run, CallId::new("c1"));
    let result = shell()
        .execute_with_context(
            json!({"command": "echo first; sleep 2; echo second", "yield_after_secs": 1}),
            ToolCallOptions::default(),
            Some(&first),
        )
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let out = result.output();
    assert!(out.starts_with("Still running as process p1"), "{out}");
    assert!(out.contains("[stdout]\nfirst"), "{out}");

    let follow = ToolExecutionContext::from_run_context(&run, CallId::new("c2"));
    let done = ShellProcessTool::new()
        .execute_with_context(
            json!({"process": "p1", "wait_secs": 10}),
            ToolCallOptions::default(),
            Some(&follow),
        )
        .await
        .unwrap();
    assert!(!done.is_error, "{}", done.output());
    let out = done.output();
    assert!(out.contains("exited with code 0"), "{out}");
    assert!(out.contains("second") && !out.contains("first"), "{out}");
}

#[cfg(not(windows))]
#[tokio::test]
async fn without_a_turn_a_command_runs_to_its_end() {
    let result = shell()
        .execute(json!({"command": "echo start; sleep 2; echo done", "yield_after_secs": 30}))
        .await
        .unwrap();
    assert_eq!(
        result.output().trim(),
        "start\ndone",
        "nothing to follow it with"
    );
}

#[cfg(not(windows))]
#[tokio::test]
async fn yield_after_zero_waits_for_the_end() {
    let run = test_turn_run();
    let call = ToolExecutionContext::from_run_context(&run, CallId::new("c1"));
    let result = shell()
        .execute_with_context(
            json!({"command": "sleep 2; echo done", "yield_after_secs": 0}),
            ToolCallOptions::default(),
            Some(&call),
        )
        .await
        .unwrap();
    assert_eq!(result.output().trim(), "done");
}

#[cfg(not(windows))]
#[tokio::test]
async fn a_call_that_cannot_yield_comes_back_by_the_models_time() {
    // A deliberate wait does not yield, so the model's `timeout_secs` stops it
    // instead, as `timeout_secs` did before it yielded.
    let run = test_turn_run();
    let call = ToolExecutionContext::from_run_context(&run, CallId::new("c1"));
    let result = shell()
        .execute_with_context(
            json!({"command": "sleep 5; tail -1 /dev/null", "yield_after_secs": 1}),
            ToolCallOptions::default(),
            Some(&call),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        result.output().starts_with("Command timed out after 1s"),
        "{}",
        result.output()
    );
}

#[cfg(not(windows))]
#[tokio::test]
async fn a_failing_end_reads_like_a_failed_shell_call() {
    let run = test_turn_run();
    let call = ToolExecutionContext::from_run_context(&run, CallId::new("c1"));
    let yielded = shell()
        .execute_with_context(
            json!({"command": "true; sleep 2; echo broke >&2; exit 3", "yield_after_secs": 1}),
            ToolCallOptions::default(),
            Some(&call),
        )
        .await
        .unwrap();
    assert!(
        yielded.output().contains("No output yet."),
        "{}",
        yielded.output()
    );
    let follow = ToolExecutionContext::from_run_context(&run, CallId::new("c2"));
    let ended = ShellProcessTool::new()
        .execute_with_context(
            json!({"process": "p1"}),
            ToolCallOptions::default(),
            Some(&follow),
        )
        .await
        .unwrap();
    assert!(ended.is_error);
    let out = ended.output();
    assert!(
        out.contains("exit code 3") && out.contains("broke"),
        "{out}"
    );
}

#[tokio::test]
async fn shell_process_without_a_turn_says_so() {
    let result = ShellProcessTool::new()
        .execute(json!({"process": "p1"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        result.output().contains("only a turn can leave"),
        "{}",
        result.output()
    );
}
