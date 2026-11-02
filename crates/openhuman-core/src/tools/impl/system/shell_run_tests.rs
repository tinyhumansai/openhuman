//! A `shell` call's run: what it asked about coming back, the sandboxed path's
//! yield, and when its audit entry is written.

use super::*;
use crate::agent::tinyagents::host::run_context::test_turn_run;
use crate::tools::implementations::system::ShellProcessTool;
use serde_json::json;
use tinyagents_harness::ids::CallId;
use tinyagents_harness::tool::ToolExecutionContext;
use tinytools::{Tool, ToolCallOptions};

#[test]
fn a_call_yields_only_when_the_turn_moved_its_timeout() {
    let none = YieldRequest::from_args(&json!({"command": "ls", "timeout_secs": 60}));
    assert!(!none.allowed);
    let default = YieldRequest::from_args(&json!({"command": "ls", YIELD_AFTER_ARG: null}));
    assert!(default.allowed);
    assert_eq!(default.requested, None);
    let given = YieldRequest::from_args(&json!({"command": "ls", YIELD_AFTER_ARG: 45}));
    assert!(given.allowed);
    assert_eq!(given.requested, Some(45));
}

#[test]
fn where_a_command_cannot_be_left_running_the_models_time_is_its_deadline() {
    let asked = YieldRequest {
        allowed: true,
        requested: Some(60),
    };
    assert_eq!(asked.deadline_instead(Some(600)), Some(60));
    assert_eq!(
        asked.deadline_instead(Some(30)),
        Some(30),
        "the turn's is sooner"
    );
    assert_eq!(asked.deadline_instead(None), Some(60));
    let default = YieldRequest {
        allowed: true,
        requested: None,
    };
    assert_eq!(default.deadline_instead(Some(600)), Some(600));
    assert_eq!(default.deadline_instead(None), None);
    let wait = YieldRequest {
        allowed: true,
        requested: Some(0),
    };
    assert_eq!(
        wait.deadline_instead(None),
        None,
        "0 means wait for the end"
    );
}

fn unsandboxed_policy() -> crate::sandbox::SandboxPolicy {
    crate::sandbox::policy::route_for_mode(
        tinyagents_definition::SandboxMode::None,
        &std::env::temp_dir(),
        &std::env::temp_dir().join("oh-shell-run-state"),
        &crate::config::SandboxConfig::default(),
        &crate::config::RuntimeConfig::default(),
        false,
    )
}

#[cfg(not(windows))]
#[tokio::test]
async fn a_sandboxed_command_yields_and_is_followed_to_its_end() {
    let run = test_turn_run();
    let call = ToolExecutionContext::from_run_context(&run, CallId::new("c1"));
    let audit = Mutex::new(None);
    let (allowed, result) = run_in_policy(
        &crate::core::runtime::DispatchCtx::unbound(),
        unsandboxed_policy(),
        "echo first; sleep 2; echo second",
        None,
        YieldRequest {
            allowed: true,
            requested: Some(1),
        },
        &std::env::temp_dir(),
        Some(&call),
        None,
        &audit,
    )
    .await;
    assert!(allowed);
    let out = result.output();
    assert!(out.starts_with("Still running as process p1"), "{out}");
    assert!(out.contains("first"), "{out}");

    let follow = ToolExecutionContext::from_run_context(&run, CallId::new("c2"));
    let done = ShellProcessTool::new()
        .execute_with_context(
            json!({"process": "p1", "wait_secs": 10}),
            ToolCallOptions::default(),
            Some(&follow),
        )
        .await
        .unwrap();
    let out = done.output();
    assert!(
        out.contains("exited with code 0") && out.contains("second"),
        "{out}"
    );
}

#[cfg(not(windows))]
#[tokio::test]
async fn without_a_turn_a_sandboxed_command_runs_to_its_end() {
    let (_, result) = run_in_policy(
        &crate::core::runtime::DispatchCtx::unbound(),
        unsandboxed_policy(),
        "echo start; sleep 2; echo done",
        None,
        YieldRequest {
            allowed: true,
            requested: Some(30),
        },
        &std::env::temp_dir(),
        None,
        None,
        &Mutex::new(None),
    )
    .await;
    assert_eq!(
        result.output().trim(),
        "start\ndone",
        "nothing to follow it with"
    );
}
