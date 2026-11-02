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

#[test]
fn a_backend_that_cannot_yield_takes_no_place() {
    let run = test_turn_run();
    let request = YieldRequest {
        allowed: true,
        requested: Some(1),
    };
    let deadline = Duration::from_secs(600);

    // A one-shot backend (Docker, the box) reserves no slot and is bounded by
    // the model's come-back time, as `timeout_secs` was before it yielded.
    let one_shot = ToolExecutionContext::from_run_context(&run, CallId::new("c1"));
    let (slot, after, bounded) = come_back(
        request,
        Some(&one_shot),
        "echo hi; sleep 2",
        deadline,
        false,
    );
    assert!(slot.is_none(), "a one-shot backend must not hold a slot");
    assert!(after.is_none(), "a one-shot backend does not yield");
    assert_eq!(
        bounded,
        Duration::from_secs(1),
        "the model's come-back time is the deadline instead"
    );

    // Every one of the turn's four slots is still free afterwards.
    let processes =
        crate::agent::tinyagents::host::run_context::shell_processes_for_tool(Some(&one_shot))
            .expect("the turn has a process registry");
    let held: Vec<_> = std::iter::from_fn(|| processes.reserve()).collect();
    assert_eq!(
        held.len(),
        crate::sandbox::shell_processes::MAX_RUNNING,
        "the one-shot call left all slots free"
    );
    drop(held);

    // A native backend reserves its slot and keeps the full deadline.
    let native = ToolExecutionContext::from_run_context(&run, CallId::new("c2"));
    let (slot, after, kept) = come_back(request, Some(&native), "echo hi; sleep 2", deadline, true);
    assert!(slot.is_some(), "a native backend reserves its slot");
    assert!(after.is_some(), "a native backend yields");
    assert_eq!(kept, deadline, "a yielding call keeps its full deadline");
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
