//! A `shell` call's run: what it asked about coming back, the sandboxed path's
//! yield, and when its audit entry is written.

use super::*;
use crate::agent::tinyagents::host::run_context::test_turn_run;
use crate::security::{AuditLogger, AutonomyLevel, SecurityPolicy};
use crate::tools::implementations::system::{ShellProcessTool, ShellTool};
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
    crate::sandbox::resolve_sandbox_policy(
        &crate::core::runtime::DispatchCtx::unbound(),
        tinyagents_definition::SandboxMode::None,
        &std::env::temp_dir(),
        &std::env::temp_dir().join("oh-shell-run-state"),
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

fn audit_log() -> (Arc<AuditLogger>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let logger = AuditLogger::new(
        crate::config::AuditConfig {
            enabled: true,
            log_path: "audit.log".into(),
            max_size_mb: 10,
        },
        tmp.path().to_path_buf(),
    )
    .expect("create audit logger");
    (Arc::new(logger), tmp)
}

fn audit_lines(dir: &tempfile::TempDir) -> Vec<crate::security::AuditEvent> {
    std::fs::read_to_string(dir.path().join("audit.log"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("audit event JSON parses"))
        .collect()
}

#[cfg(not(windows))]
#[tokio::test]
async fn a_command_left_running_is_audited_when_it_ends() {
    let (audit, dir) = audit_log();
    let shell = ShellTool::new(
        Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Full,
            workspace_dir: std::env::temp_dir(),
            action_dir: std::env::temp_dir(),
            ..SecurityPolicy::default()
        }),
        Arc::new(crate::modules::tinybox::host_runtime::NativeRuntime::new()),
        audit,
    );
    let run = test_turn_run();
    let call = ToolExecutionContext::from_run_context(&run, CallId::new("c1"));
    let yielded = shell
        .execute_with_context(
            json!({"command": "echo a; sleep 2; exit 3", YIELD_AFTER_ARG: 1}),
            ToolCallOptions::default(),
            Some(&call),
        )
        .await
        .unwrap();
    assert!(
        yielded.output().starts_with("Still running"),
        "{}",
        yielded.output()
    );
    assert!(audit_lines(&dir).is_empty(), "no outcome is known yet");

    let follow = ToolExecutionContext::from_run_context(&run, CallId::new("c2"));
    ShellProcessTool::new()
        .execute_with_context(
            json!({"process": "p1", "wait_secs": 10}),
            ToolCallOptions::default(),
            Some(&follow),
        )
        .await
        .unwrap();
    let lines = audit_lines(&dir);
    assert_eq!(lines.len(), 1, "one entry, written at the end");
    let result = lines[0].result.as_ref().expect("result present");
    assert!(!result.success, "exit 3 is audited as a failure");
}

/// #8317: a backend that cannot leave a command running takes no place in the
/// turn, so native commands can still use all four; its run is bounded by the
/// model's time instead.
#[test]
fn a_backend_that_cannot_yield_takes_no_place() {
    use crate::agent::tinyagents::host::run_context::shell_processes_for_tool;
    use crate::sandbox::shell_processes::MAX_RUNNING;

    let run = test_turn_run();
    let call = ToolExecutionContext::from_run_context(&run, CallId::new("c1"));
    let request = YieldRequest {
        allowed: true,
        requested: Some(60),
    };
    let deadline = Duration::from_secs(600);

    let (slot, after, bound) = come_back(request, Some(&call), "make", deadline, false);
    assert!(slot.is_none(), "a one-shot backend reserves nothing");
    assert_eq!(after, None);
    assert_eq!(bound, Duration::from_secs(60), "the model's time bounds it");
    let processes = shell_processes_for_tool(Some(&call)).expect("a turn has a registry");
    let held: Vec<_> = (0..MAX_RUNNING)
        .map(|_| processes.reserve().expect("all four places are free"))
        .collect();
    drop(held);

    let (slot, after, bound) = come_back(request, Some(&call), "make", deadline, true);
    assert!(slot.is_some(), "a native backend reserves its place");
    assert_eq!(after, Some(Duration::from_secs(60)));
    assert_eq!(
        bound, deadline,
        "a command it may leave running keeps its deadline"
    );
}
