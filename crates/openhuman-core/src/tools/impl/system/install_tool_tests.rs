use super::*;
use crate::security::approval::ApprovalChatContext;
use crate::security::AutonomyLevel;
use serde_json::json;
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::sync::Mutex;
use tinyagents_harness_tools::exec::{
    ExecBackendOutcome, ExecOutput, ExecProgress, ExecRequest, ShellFlavor,
};

#[derive(Default)]
struct RecordingBackend {
    environment: Mutex<Option<BTreeMap<OsString, OsString>>>,
}

#[async_trait::async_trait]
impl ExecBackend for RecordingBackend {
    fn shell_flavor(&self) -> ShellFlavor {
        ShellFlavor::Posix
    }

    async fn execute(
        &self,
        request: ExecRequest,
        _context: Option<&dyn tinytools::ToolRunContext>,
        _progress: Option<ExecProgress>,
    ) -> ExecBackendOutcome {
        *self.environment.lock().unwrap() = Some(request.environment);
        ExecBackendOutcome::Completed(ExecOutput {
            stdout: Vec::new(),
            stderr: Vec::new(),
            exit_code: Some(0),
            timed_out: false,
        })
    }
}

fn policy(allow_install: bool, autonomy: AutonomyLevel) -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy {
        autonomy,
        allow_tool_install: allow_install,
        workspace_dir: std::env::temp_dir(),
        ..SecurityPolicy::default()
    })
}

// install_tool refuses outside an interactive (approval) turn — Gate 0 — so
// tests that exercise the *other* gates run inside a chat context.
fn chat_ctx() -> ApprovalChatContext {
    ApprovalChatContext {
        thread_id: "t-test".into(),
        client_id: "c-test".into(),
        request_id: None,
    }
}

/// The execution context the harness hands a tool dispatched inside a turn
/// whose chat is `chat`.
fn chat_tool_context(
    chat: crate::security::approval::ApprovalChatContext,
) -> tinyagents_harness::tool::ToolExecutionContext {
    crate::agent::tinyagents::host::run_context::tool_context_with_approval(
        crate::security::approval::ApprovalTurnContext::chat(chat),
    )
}

#[tokio::test]
async fn blocked_when_install_disabled() {
    let tool = InstallToolTool::new(policy(false, AutonomyLevel::Full));
    let result = tool
        .execute_with_context(
            json!({ "package": "jq" }),
            tinytools::ToolCallOptions::default(),
            Some(&chat_tool_context(chat_ctx())),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("disabled"), "{}", result.output());
}

#[tokio::test]
async fn blocked_when_readonly() {
    let tool = InstallToolTool::new(policy(true, AutonomyLevel::ReadOnly));
    let result = tool
        .execute_with_context(
            json!({ "package": "jq" }),
            tinytools::ToolCallOptions::default(),
            Some(&chat_tool_context(chat_ctx())),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("read-only"), "{}", result.output());
}

#[tokio::test]
async fn rejects_injection_in_package_name() {
    let tool = InstallToolTool::new(policy(true, AutonomyLevel::Full));
    let result = tool
        .execute_with_context(
            json!({ "package": "jq; rm -rf /" }),
            tinytools::ToolCallOptions::default(),
            Some(&chat_tool_context(chat_ctx())),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        result.output().contains("Invalid package name"),
        "{}",
        result.output()
    );
}

#[tokio::test]
async fn refuses_in_autonomous_turn_without_chat_context() {
    // No chat approval context → background/autonomous turn → refused
    // by Gate 0 before any install logic runs.
    let tool = InstallToolTool::new(policy(true, AutonomyLevel::Full));
    let result = tool.execute(json!({ "package": "jq" })).await.unwrap();
    assert!(result.is_error);
    assert!(
        result.output().contains("interactive approval"),
        "{}",
        result.output()
    );
}

#[tokio::test]
async fn rejects_unknown_manager() {
    let tool = InstallToolTool::new(policy(true, AutonomyLevel::Full));
    let result = tool
        .execute(json!({ "package": "jq", "manager": "notamanager" }))
        .await
        .unwrap();
    assert!(result.is_error);
}

#[tokio::test]
async fn install_backend_receives_only_the_explicit_install_environment() {
    let expected = BTreeMap::from([
        (OsString::from("PATH"), OsString::from("/toolchain/bin")),
        (OsString::from("HOME"), OsString::from("/home/operator")),
        (
            OsString::from("CARGO_HOME"),
            OsString::from("/home/operator/.cargo"),
        ),
    ]);
    let backend = Arc::new(RecordingBackend::default());
    let tool = InstallToolTool::with_backend_and_environment(
        policy(true, AutonomyLevel::Full),
        AuditSink::disabled(),
        Arc::new(crate::config::Config::default()),
        backend.clone(),
        expected.clone(),
    );

    let result = tool
        .execute_with_context(
            json!({ "package": "fixture-package", "manager": "cargo" }),
            tinytools::ToolCallOptions::default(),
            Some(&chat_tool_context(chat_ctx())),
        )
        .await
        .unwrap();

    assert!(!result.is_error, "{}", result.output());
    assert_eq!(
        backend.environment.lock().unwrap().as_ref(),
        Some(&expected),
        "the installer backend must receive the host-approved environment"
    );
}

#[test]
fn install_environment_filters_host_variables_without_dropping_package_manager_paths() {
    let filtered = filter_install_environment([
        (OsString::from("PATH"), OsString::from("/tools/bin")),
        (OsString::from("HOME"), OsString::from("/home/operator")),
        (
            OsString::from("CARGO_HOME"),
            OsString::from("/home/operator/.cargo"),
        ),
        (OsString::from("SystemRoot"), OsString::from("C:\\Windows")),
        (OsString::from("GH_TOKEN"), OsString::from("secret")),
        (
            OsString::from("AWS_ACCESS_KEY_ID"),
            OsString::from("secret"),
        ),
    ]);

    assert_eq!(
        filtered.get(OsStr::new("PATH")),
        Some(&OsString::from("/tools/bin"))
    );
    assert_eq!(
        filtered.get(OsStr::new("HOME")),
        Some(&OsString::from("/home/operator"))
    );
    assert_eq!(
        filtered.get(OsStr::new("CARGO_HOME")),
        Some(&OsString::from("/home/operator/.cargo"))
    );
    assert_eq!(
        filtered.get(OsStr::new("SystemRoot")),
        Some(&OsString::from("C:\\Windows"))
    );
    assert!(!filtered.contains_key(OsStr::new("GH_TOKEN")));
    assert!(!filtered.contains_key(OsStr::new("AWS_ACCESS_KEY_ID")));
}
