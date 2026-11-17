use super::*;

use crate::flows::builder_tools_connection_reads::flow_connection_to_json;
// ── revise_workflow ──────────────────────────────────────────────────────────

#[tokio::test]
async fn revise_workflow_validates_and_returns_revision_proposal() {
    let tmp = TempDir::new().unwrap();
    let tool = revise_workflow_tool(test_config(&tmp));

    let result = tool
        .execute(json!({
            "name": "Revised flow",
            "graph": valid_graph(),
            "instruction": "add a summarize step"
        }))
        .await
        .unwrap();

    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["type"], "workflow_proposal");
    assert_eq!(parsed["revision"], true);
    assert_eq!(parsed["name"], "Revised flow");
    assert_eq!(parsed["instruction"], "add a summarize step");
    assert_eq!(parsed["graph"]["nodes"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn revise_workflow_omitted_require_approval_defaults_true() {
    let tmp = TempDir::new().unwrap();
    let tool = revise_workflow_tool(test_config(&tmp));

    let result = tool
        .execute(json!({ "name": "Revised flow", "graph": valid_graph() }))
        .await
        .unwrap();

    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["require_approval"], true);
}

#[tokio::test]
async fn revise_workflow_explicit_require_approval_true_is_respected() {
    let tmp = TempDir::new().unwrap();
    let tool = revise_workflow_tool(test_config(&tmp));

    let result = tool
        .execute(json!({
            "name": "Revised flow",
            "graph": valid_graph(),
            "require_approval": true
        }))
        .await
        .unwrap();

    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["require_approval"], true);
}

#[tokio::test]
async fn revise_workflow_rejects_invalid_graph() {
    let tmp = TempDir::new().unwrap();
    let tool = revise_workflow_tool(test_config(&tmp));

    let result = tool
        .execute(json!({
            "name": "bad",
            "graph": { "nodes": [ { "id": "a", "kind": "agent", "name": "A" } ], "edges": [] }
        }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.output().to_lowercase().contains("invalid"));
}

#[test]
fn revise_workflow_never_persists() {
    // The revise tool shares propose_workflow's human-in-the-loop invariant:
    // no side effect, no permission gate — it only validates and returns.
    let tmp = TempDir::new().unwrap();
    let tool = revise_workflow_tool(test_config(&tmp));
    assert_eq!(tool.name(), "revise_workflow");
    assert_eq!(tool.permission_level(), PermissionLevel::None);
    assert!(!tool.external_effect());
}

// ── read-only tools ──────────────────────────────────────────────────────────

#[tokio::test]
async fn list_flows_is_read_only_and_lists() {
    let tmp = TempDir::new().unwrap();
    let tool = list_flows_tool(test_config(&tmp));
    assert_eq!(tool.permission_level(), PermissionLevel::None);
    assert!(!tool.external_effect());

    let result = tool.execute(json!({})).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    // No flows saved in a fresh workspace.
    assert!(parsed["flows"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn get_flow_missing_id_is_error() {
    let tmp = TempDir::new().unwrap();
    let tool = get_flow_tool(test_config(&tmp));
    assert_eq!(tool.permission_level(), PermissionLevel::None);

    let result = tool.execute(json!({})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Missing 'id'"));
}

#[tokio::test]
async fn get_flow_unknown_id_is_error() {
    let tmp = TempDir::new().unwrap();
    let tool = get_flow_tool(test_config(&tmp));

    let result = tool.execute(json!({ "id": "nope" })).await.unwrap();
    assert!(result.is_error);
    assert!(
        result.output().to_lowercase().contains("not found") || result.output().contains("nope")
    );
}

#[tokio::test]
async fn get_flow_run_missing_id_is_error() {
    let tmp = TempDir::new().unwrap();
    let tool = get_flow_run_tool(test_config(&tmp));
    assert_eq!(tool.permission_level(), PermissionLevel::None);

    let result = tool.execute(json!({})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Missing 'run_id'"));
}

#[tokio::test]
async fn list_flow_connections_is_read_only() {
    // Reaches the connector module, whose route is process-wide: hold the module
    // lock (before TEST_ENV_LOCK, the established order) for the whole test.
    let _module = crate::integrations::composio::module_client::module_guard().await;
    let tmp = TempDir::new().unwrap();
    let tool = ListFlowConnectionsTool::new(
        test_config(&tmp),
        Arc::new(crate::flows::builder_tools_connection_reads::FlowsConnectionProvider),
    );
    assert_eq!(tool.permission_level(), PermissionLevel::None);
    assert!(!tool.external_effect());

    let result = tool.execute(json!({})).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert!(parsed["connections"].is_array());
}

#[test]
fn list_flow_connections_json_surfaces_platform_user_id() {
    use crate::flows::ops::FlowConnection;

    let with_identity = FlowConnection {
        connection_ref: "composio:slack:ca_slack1".to_string(),
        kind: "composio".to_string(),
        display: "Slack".to_string(),
        toolkit: Some("slack".to_string()),
        scheme: None,
        platform_user_id: Some("U123ABC".to_string()),
    };
    let json = flow_connection_to_json(&with_identity);
    assert_eq!(json["platform_user_id"], "U123ABC");

    let without_identity = FlowConnection {
        platform_user_id: None,
        ..with_identity
    };
    let json = flow_connection_to_json(&without_identity);
    assert!(json["platform_user_id"].is_null());
}
