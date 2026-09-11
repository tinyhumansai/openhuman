use super::*;
use crate::openhuman::security::approval::{ApprovalChatContext, APPROVAL_CHAT_CONTEXT};
use crate::openhuman::security::AutonomyLevel;

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
    }
}

#[tokio::test]
async fn blocked_when_install_disabled() {
    let tool = InstallToolTool::new(policy(false, AutonomyLevel::Full));
    let result = APPROVAL_CHAT_CONTEXT
        .scope(chat_ctx(), tool.execute(json!({ "package": "jq" })))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("disabled"), "{}", result.output());
}

#[tokio::test]
async fn blocked_when_readonly() {
    let tool = InstallToolTool::new(policy(true, AutonomyLevel::ReadOnly));
    let result = APPROVAL_CHAT_CONTEXT
        .scope(chat_ctx(), tool.execute(json!({ "package": "jq" })))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("read-only"), "{}", result.output());
}

#[tokio::test]
async fn rejects_injection_in_package_name() {
    let tool = InstallToolTool::new(policy(true, AutonomyLevel::Full));
    let result = APPROVAL_CHAT_CONTEXT
        .scope(
            chat_ctx(),
            tool.execute(json!({ "package": "jq; rm -rf /" })),
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
    // No APPROVAL_CHAT_CONTEXT scope → background/autonomous turn → refused
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

#[test]
fn package_name_validation() {
    assert!(is_valid_package_name("ripgrep"));
    assert!(is_valid_package_name("@scope/cli"));
    assert!(is_valid_package_name("python3.11"));
    assert!(!is_valid_package_name("jq; rm -rf /"));
    assert!(!is_valid_package_name("a b"));
    assert!(!is_valid_package_name(""));
}
