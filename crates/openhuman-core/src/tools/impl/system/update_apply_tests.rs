use super::*;
use crate::security::{AutonomyLevel, SecurityPolicy};
use serde_json::json;
use std::sync::Arc;

fn supervised() -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        workspace_dir: std::env::temp_dir(),
        ..SecurityPolicy::default()
    })
}

fn read_only() -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        workspace_dir: std::env::temp_dir(),
        ..SecurityPolicy::default()
    })
}

#[test]
fn name_and_permission() {
    let tool = UpdateApplyTool::new(supervised());
    assert_eq!(tool.name(), "update_apply");
    assert_eq!(tool.permission_level(), PermissionLevel::Dangerous);
}

#[test]
fn schema_requires_user_confirmed_boolean() {
    let schema = UpdateApplyTool::new(supervised()).parameters_schema();
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["required"][0], "user_confirmed");
    assert_eq!(schema["properties"]["user_confirmed"]["type"], "boolean");
}

#[test]
fn description_calls_out_consent_and_companion_check_tool() {
    let tool = UpdateApplyTool::new(supervised());
    let desc = tool.description();
    assert!(desc.contains("ask_user_clarification"));
    assert!(desc.contains("update_check"));
    assert!(desc.contains("user_confirmed"));
    assert!(desc.contains("HIGH IMPACT"));
}

#[tokio::test]
async fn rejects_when_user_confirmed_missing() {
    let tool = UpdateApplyTool::new(supervised());
    let result = tool.execute(json!({})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("user consent"));
    assert!(result.output().contains("ask_user_clarification"));
}

#[tokio::test]
async fn rejects_when_user_confirmed_false() {
    let tool = UpdateApplyTool::new(supervised());
    let result = tool
        .execute(json!({ "user_confirmed": false }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("user consent"));
}

/// The tool no longer checks its own tier: refusing a read-only session is the
/// host authorization stage's job. `update_apply` replaces the running binary,
/// so it has to declare itself as `Dangerous` for that stage to refuse it.
// Fails closed only while security is enforced: a build without the
// security module has security disabled (`crate::security::switch`).
#[cfg(feature = "security-module")]
#[tokio::test]
async fn the_host_stage_rejects_it_in_read_only_mode_even_with_consent() {
    use crate::agent::tinyagents::host::security_gate_test_support::{authorize_one, is_denied};
    let tool = UpdateApplyTool::new(read_only());
    assert_eq!(tool.permission_level(), PermissionLevel::Dangerous);
    let decision = authorize_one(
        read_only(),
        Box::new(tool),
        json!({ "user_confirmed": true }),
    )
    .await;
    assert!(
        is_denied(&decision),
        "a read-only session must not apply an update; got {decision:?}"
    );
}

#[tokio::test]
async fn consent_check_runs_before_autonomy_check() {
    // A read-only session that also forgot to confirm should see the
    // consent error first — this keeps the LLM-facing failure mode
    // consistent regardless of the host autonomy level (a downgraded
    // session shouldn't suddenly mint different "fix this" hints).
    let tool = UpdateApplyTool::new(read_only());
    let result = tool.execute(json!({})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("user consent"));
    assert!(!result.output().contains("read-only mode"));
}
