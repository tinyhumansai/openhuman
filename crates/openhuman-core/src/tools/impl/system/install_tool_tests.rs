use super::*;
use crate::security::approval::ApprovalChatContext;
use crate::security::AutonomyLevel;

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
async fn refuses_host_package_manager_execution_until_an_isolated_installer_exists() {
    let tool = InstallToolTool::new(policy(true, AutonomyLevel::Full));
    let result = tool
        .execute_with_context(
            json!({ "package": "jq", "manager": "apt" }),
            tinytools::ToolCallOptions::default(),
            Some(&chat_tool_context(chat_ctx())),
        )
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(
        result.output().contains("isolated package installer"),
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

#[test]
fn saas_refuses_installs_and_single_user_hosts_do_not() {
    let refusal = refuse_under_saas(true).expect("SaaS must refuse");
    assert!(refusal.is_error);
    assert!(refusal.output().contains("SaaS"), "{}", refusal.output());
    assert!(refuse_under_saas(false).is_none());
}
