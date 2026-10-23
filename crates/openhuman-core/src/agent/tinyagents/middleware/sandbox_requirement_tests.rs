use super::*;

use tinyagents_harness::context::{RunConfig, RunContext};

fn policy(sandbox: SandboxMode) -> SandboxRequirementPolicy {
    SandboxRequirementPolicy::new(HashMap::from([(
        "send_email".into(),
        ToolPolicy::classified().with_runtime(tinytools::ToolRuntime {
            sandbox,
            ..tinytools::ToolRuntime::default()
        }),
    )]))
}

fn call(name: &str) -> TaToolCall {
    TaToolCall {
        id: "c1".into(),
        name: name.into(),
        arguments: serde_json::json!({}),
        invalid: None,
    }
}

fn context() -> RunContext<OpenHumanRunContext> {
    RunContext::new(
        RunConfig::new("sandbox-requirement"),
        OpenHumanRunContext::new(),
    )
}

#[tokio::test]
async fn sandbox_required_tool_is_denied_without_a_sandboxed_workspace() {
    let decision = policy(SandboxMode::Required)
        .check(&context(), &call("send_email"))
        .await;

    assert!(
        matches!(decision, PolicyDecision::Deny { reason } if reason.contains("requires a sandbox"))
    );
}

#[tokio::test]
async fn sandbox_required_tool_is_allowed_in_a_sandboxed_workspace() {
    let ctx = context().with_workspace(
        tinytools::WorkspaceDescriptor::new("/workspace").with_sandbox(SandboxMode::Required),
    );
    let decision = policy(SandboxMode::Required)
        .check(&ctx, &call("send_email"))
        .await;

    assert_eq!(decision, PolicyDecision::Allow);
}

#[tokio::test]
async fn tools_without_a_sandbox_requirement_are_not_restricted() {
    let decision = policy(SandboxMode::Disabled)
        .check(&context(), &call("send_email"))
        .await;

    assert_eq!(decision, PolicyDecision::Allow);
}
