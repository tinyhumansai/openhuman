use super::*;
use crate::core::runtime::DispatchCtx;

#[test]
fn every_registered_controller_is_in_the_mcp_ui_namespace() {
    let controllers = all_registered_controllers();
    assert_eq!(controllers.len(), all_controller_schemas().len());
    for controller in controllers {
        assert_eq!(controller.schema.namespace, "mcp_ui");
        assert_ne!(controller.schema.function, "unknown");
    }
}

#[test]
fn tool_call_requires_server_and_tool() {
    let schema = schemas("tool_call");
    let required: Vec<_> = schema
        .inputs
        .iter()
        .filter(|field| field.required)
        .map(|field| field.name)
        .collect();
    assert_eq!(required, vec!["server_id", "tool_name"]);
}

async fn with_workspace<F: std::future::Future>(body: F) -> F::Output {
    let _env_lock = crate::config::TEST_ENV_LOCK.lock().await;
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_env = crate::config::test_env::EnvVarGuard::workspace_unlocked(workspace.path());
    body.await
}

fn params(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

#[tokio::test]
async fn resource_read_handler_rejects_a_missing_server() {
    let error = with_workspace(handle_resource_read(
        DispatchCtx::process_default(),
        params(serde_json::json!({})),
    ))
    .await
    .unwrap_err();
    assert!(error.contains("server_id"));
}

#[tokio::test]
async fn resource_read_handler_rejects_an_unknown_inline_id() {
    let error = with_workspace(handle_resource_read(
        DispatchCtx::process_default(),
        params(serde_json::json!({"inline_id": "absent"})),
    ))
    .await
    .unwrap_err();
    assert!(error.contains("no longer available"));
}

#[tokio::test]
async fn tool_call_handler_rejects_a_missing_server() {
    let error = with_workspace(handle_tool_call(
        DispatchCtx::process_default(),
        params(serde_json::json!({})),
    ))
    .await
    .unwrap_err();
    assert!(error.contains("server_id"));
}

#[tokio::test]
async fn tool_call_handler_refuses_an_unavailable_server() {
    let result = with_workspace(handle_tool_call(
        DispatchCtx::process_default(),
        params(serde_json::json!({"server_id": "nowhere", "tool_name": "order"})),
    ))
    .await;
    assert!(result.is_err());
}
