use super::*;
use crate::security::AutonomyLevel;

#[test]
fn update_family_registration_preserves_tool_names_and_order() {
    let tools = registered_tools(std::sync::Arc::new(SecurityPolicy::default()));
    let names = tools.iter().map(|tool| tool.name()).collect::<Vec<_>>();

    assert_eq!(names, ["update_check", "update_apply"]);
}

#[test]
fn update_check_name_and_permission() {
    let tool = UpdateCheckTool::new();
    assert_eq!(tool.name(), "update_check");
    assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
}

#[test]
fn schema_is_closed_object_with_no_properties() {
    let schema = UpdateCheckTool::new().parameters_schema();
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["additionalProperties"], false);
    let props = schema["properties"].as_object().expect("properties object");
    assert!(
        props.is_empty(),
        "update_check takes no arguments — schema must not advertise any"
    );
}

#[test]
fn description_mentions_safety_and_companion_tool() {
    let tool = UpdateCheckTool::new();
    let desc = tool.description();
    assert!(desc.contains("Read-only"));
    assert!(desc.contains("update_apply"));
    assert!(desc.contains("Does NOT download"));
}

#[test]
fn default_constructs_same_as_new() {
    let a = UpdateCheckTool;
    let b = UpdateCheckTool::new();
    assert_eq!(a.name(), b.name());
}
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
fn update_apply_name_and_permission() {
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

#[tokio::test]
async fn rejects_in_read_only_mode_even_with_consent() {
    let tool = UpdateApplyTool::new(read_only());
    let result = tool
        .execute(json!({ "user_confirmed": true }))
        .await
        .unwrap();
    assert!(result.is_error);
    // The autonomy gate now delegates to `SecurityPolicy::enforce_tool_operation`,
    // whose canonical read-only message is `"Security policy: read-only
    // mode, cannot perform '<op_name>'"` — assert against that phrasing
    // and the operation name rather than the old hand-rolled string.
    let body = result.output();
    assert!(
        body.contains("read-only mode") && body.contains("update_apply"),
        "expected shared enforcer's read-only message, got: {body}"
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
