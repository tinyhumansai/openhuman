//! Unit tests for `WorkflowLoadTool` and `WorkflowPhaseTool`.
//!
//! Focus on:
//! * Tool metadata (name, description, permission_level, external_effect).
//! * `parameters_schema` shape.
//! * Argument validation (missing/invalid args return an error ToolResult).
//! * Error path when a workflow id does not resolve.
//!
//! Running the actual shell in a unit test is optional and deliberately
//! omitted — the approval-gate and security-policy paths are tested in the
//! integration suite.

use super::*;
use crate::openhuman::agent::host_runtime::NativeRuntime;
use crate::openhuman::security::SecurityPolicy;

fn make_phase_tool() -> WorkflowPhaseTool {
    WorkflowPhaseTool::new(
        std::path::PathBuf::from("."),
        Arc::new(SecurityPolicy::default()),
        Arc::new(NativeRuntime::new()),
        crate::openhuman::security::AuditLogger::disabled(),
    )
}

// ── WorkflowLoadTool metadata ────────────────────────────────────────────────

#[test]
fn load_tool_name() {
    assert_eq!(WorkflowLoadTool.name(), "workflow_load");
}

#[test]
fn load_tool_is_readonly() {
    assert_eq!(
        WorkflowLoadTool.permission_level(),
        PermissionLevel::ReadOnly
    );
}

#[test]
fn load_tool_no_external_effect() {
    assert!(!WorkflowLoadTool.external_effect());
}

#[test]
fn load_tool_schema_has_required_id() {
    let schema = WorkflowLoadTool.parameters_schema();
    assert_eq!(schema["type"], "object");
    let required = schema["required"].as_array().unwrap();
    assert!(required.iter().any(|v| v.as_str() == Some("id")));
    assert!(schema["properties"]["id"].is_object());
}

// ── WorkflowPhaseTool metadata ───────────────────────────────────────────────

#[test]
fn phase_tool_name() {
    let tool = make_phase_tool();
    assert_eq!(tool.name(), "workflow_phase");
}

#[test]
fn phase_tool_is_execute() {
    let tool = make_phase_tool();
    assert_eq!(tool.permission_level(), PermissionLevel::Execute);
}

#[test]
fn phase_tool_has_external_effect() {
    let tool = make_phase_tool();
    assert!(tool.external_effect());
    assert!(tool.external_effect_with_args(&serde_json::json!({})));
}

#[test]
fn phase_tool_schema_has_required_id_and_phase() {
    let tool = make_phase_tool();
    let schema = tool.parameters_schema();
    assert_eq!(schema["type"], "object");
    let required = schema["required"].as_array().unwrap();
    let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    assert!(names.contains(&"id"), "schema must require 'id'");
    assert!(names.contains(&"phase"), "schema must require 'phase'");
    assert!(schema["properties"]["id"].is_object());
    assert!(schema["properties"]["phase"].is_object());
}

// ── WorkflowLoadTool argument validation ─────────────────────────────────────

#[tokio::test]
async fn load_missing_id_returns_error() {
    let result = WorkflowLoadTool
        .execute(serde_json::json!({}))
        .await
        .unwrap();
    assert!(result.is_error, "expected error for missing 'id'");
    assert!(result.output().contains("id"));
}

#[tokio::test]
async fn load_unknown_workflow_returns_error() {
    let result = WorkflowLoadTool
        .execute(serde_json::json!({"id": "__nonexistent_workflow_xyz__"}))
        .await
        .unwrap();
    assert!(
        result.is_error,
        "expected error for unknown workflow: {}",
        result.output()
    );
}

// ── WorkflowPhaseTool argument validation ────────────────────────────────────

#[tokio::test]
async fn phase_missing_id_returns_error() {
    let tool = make_phase_tool();
    let result = tool
        .execute(serde_json::json!({"phase": "on_pick_up_task"}))
        .await
        .unwrap();
    assert!(result.is_error, "expected error for missing 'id'");
    assert!(result.output().contains("id"));
}

#[tokio::test]
async fn phase_missing_phase_returns_error() {
    let tool = make_phase_tool();
    let result = tool
        .execute(serde_json::json!({"id": "some-workflow"}))
        .await
        .unwrap();
    assert!(result.is_error, "expected error for missing 'phase'");
    assert!(result.output().contains("phase"));
}

#[tokio::test]
async fn phase_unknown_workflow_returns_error() {
    let tool = make_phase_tool();
    let result = tool
        .execute(serde_json::json!({
            "id": "__nonexistent_workflow_xyz__",
            "phase": "on_pick_up_task"
        }))
        .await
        .unwrap();
    assert!(
        result.is_error,
        "expected error for unknown workflow: {}",
        result.output()
    );
}
