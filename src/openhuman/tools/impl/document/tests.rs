//! Unit tests for the `generate_document` tool.
//!
//! The engine layer (`engine.rs`) ships its own focused tests covering
//! the schema → OOXML mapping, the zip round-trip, empty-filtering, and
//! timeout handling. The tests here cover the tool-level concerns: input
//! validation rejection branches, the parameters schema contract, the
//! `description` router rules, and the happy-path output shape + artifact
//! finalisation (id + path + section count + size, file on disk).
//!
//! No mocks — the real `docx-rs` engine runs every test, so the happy
//! path doubles as a contract check that generation produces a valid,
//! openable `.docx` from the tool's perspective.

use super::types::{DocumentError, MAX_SECTIONS, MAX_TEXT_CHARS};
use super::*;

use std::path::Path;

fn workspace() -> tempfile::TempDir {
    tempfile::tempdir().expect("create temp workspace")
}

/// A permissive policy rooted at `workspace`. The current engine never
/// reads it, but the constructor takes it for parity with the
/// presentation tool.
fn test_security(workspace: &Path) -> Arc<SecurityPolicy> {
    use crate::openhuman::security::AutonomyLevel;
    Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Full,
        workspace_dir: workspace.to_path_buf(),
        action_dir: workspace.to_path_buf(),
        workspace_only: false,
        forbidden_paths: vec![],
        ..SecurityPolicy::default()
    })
}

fn make_tool(workspace: &Path) -> DocumentTool {
    DocumentTool::new(workspace.to_path_buf(), test_security(workspace))
}

fn minimal_input_json() -> serde_json::Value {
    json!({
        "title": "Project Charter",
        "sections": [
            { "heading": "Overview", "paragraphs": ["The plan in brief."], "bullets": ["Ship v1"] }
        ]
    })
}

/// Pull the JSON payload out of a tool result.
fn payload_of(result: &ToolResult) -> serde_json::Value {
    match result.content.first().expect("a content block") {
        crate::openhuman::skills::types::ToolContent::Json { data } => data.clone(),
        other => panic!("expected Json content block, got {other:?}"),
    }
}

#[test]
fn parameters_schema_shape_matches_contract() {
    let tool = make_tool(Path::new("/tmp/never-read"));
    let schema = tool.parameters_schema();
    assert_eq!(schema["type"], "object");
    let required = schema["required"].as_array().expect("required is array");
    assert!(required.iter().any(|v| v.as_str() == Some("title")));
    assert!(required.iter().any(|v| v.as_str() == Some("sections")));
    assert_eq!(schema["additionalProperties"], false);
    let title_props = &schema["properties"]["title"];
    assert_eq!(title_props["type"], "string");
    assert_eq!(title_props["maxLength"], MAX_TEXT_CHARS);
    let sections = &schema["properties"]["sections"];
    assert_eq!(sections["minItems"], 1);
    assert_eq!(sections["maxItems"], MAX_SECTIONS);
    let section_item = &sections["items"];
    assert_eq!(section_item["additionalProperties"], false);
}

#[test]
fn permission_level_is_write() {
    let tool = make_tool(Path::new("/tmp/never-read"));
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
}

#[test]
fn description_includes_router_rules() {
    let tool = make_tool(Path::new("/tmp/never-read"));
    let desc = tool.description();
    assert!(desc.contains("USE THIS"));
    assert!(desc.contains("NOT for"));
    assert!(desc.contains("document") || desc.contains("docx"));
}

#[tokio::test]
async fn execute_rejects_empty_title() {
    let ws = workspace();
    let tool = make_tool(ws.path());
    let args = json!({ "title": "", "sections": [{ "heading": "x", "paragraphs": ["y"] }] });
    let result = tool.execute(args).await.expect("execute returns Ok");
    assert!(result.is_error);
    assert!(result.text().contains("title"));
}

#[tokio::test]
async fn execute_rejects_empty_sections_array() {
    let ws = workspace();
    let tool = make_tool(ws.path());
    let args = json!({ "title": "Doc", "sections": [] });
    let result = tool.execute(args).await.expect("execute returns Ok");
    assert!(result.is_error);
    assert!(result.text().contains("section"));
}

#[tokio::test]
async fn execute_rejects_section_with_no_content() {
    let ws = workspace();
    let tool = make_tool(ws.path());
    let args = json!({
        "title": "Doc",
        "sections": [{ "heading": "", "paragraphs": ["   "], "bullets": [] }]
    });
    let result = tool.execute(args).await.expect("execute returns Ok");
    assert!(result.is_error);
}

#[tokio::test]
async fn execute_rejects_oversize_heading() {
    let ws = workspace();
    let tool = make_tool(ws.path());
    let big = "x".repeat(MAX_TEXT_CHARS + 1);
    let args = json!({
        "title": "Doc",
        "sections": [{ "heading": big, "paragraphs": ["ok"] }]
    });
    let result = tool.execute(args).await.expect("execute returns Ok");
    assert!(result.is_error);
}

#[tokio::test]
async fn execute_rejects_too_many_sections() {
    let ws = workspace();
    let tool = make_tool(ws.path());
    let sections: Vec<_> = (0..(MAX_SECTIONS + 1))
        .map(|i| json!({ "heading": format!("S{i}"), "paragraphs": ["x"] }))
        .collect();
    let args = json!({ "title": "Big doc", "sections": sections });
    let result = tool.execute(args).await.expect("execute returns Ok");
    assert!(result.is_error);
    assert!(result.text().contains(&MAX_SECTIONS.to_string()));
}

#[tokio::test]
async fn execute_rejects_unknown_field() {
    // `deny_unknown_fields` on the input structs means a stray key is a
    // deserialisation error surfaced as a tool error, not a silent drop.
    let ws = workspace();
    let tool = make_tool(ws.path());
    let args = json!({ "title": "Doc", "sections": [{ "paragraphs": ["x"] }], "bogus": 1 });
    let result = tool.execute(args).await.expect("execute returns Ok");
    assert!(result.is_error);
}

#[tokio::test]
async fn execute_happy_path_returns_artifact_metadata() {
    // End-to-end: drives the real docx-rs engine + artifact pipeline.
    // Asserts the success contract — the artifact is finalised on disk and
    // the markdown reply quotes the path + id.
    let ws = workspace();
    let tool = make_tool(ws.path());
    let result = tool
        .execute(minimal_input_json())
        .await
        .expect("execute returns Ok");

    assert!(
        !result.is_error,
        "happy path should not be flagged as error"
    );

    let payload = payload_of(&result);
    assert_eq!(payload["section_count"].as_u64(), Some(1));
    let artifact_path = payload["artifact_path"]
        .as_str()
        .expect("artifact_path is a string");
    let artifact_id = payload["artifact_id"]
        .as_str()
        .expect("artifact_id is a string");
    let size_bytes = payload["size_bytes"]
        .as_u64()
        .expect("size_bytes is an integer");

    assert!(
        std::path::Path::new(artifact_path).exists(),
        "artifact file must exist at {artifact_path}"
    );
    assert!(
        artifact_path.ends_with(".docx"),
        "artifact should be a .docx: {artifact_path}"
    );
    assert!(
        size_bytes > 200,
        "document unexpectedly small ({size_bytes} bytes)"
    );

    // The written file is a real, openable OOXML zip.
    let bytes = std::fs::read(artifact_path).expect("artifact file readable");
    assert_eq!(&bytes[0..2], b"PK", "artifact must be a zip (PK magic)");

    let md = result
        .markdown_formatted
        .as_deref()
        .expect("success_with_markdown sets markdown_formatted");
    assert!(md.contains(artifact_id));
    assert!(md.contains(artifact_path));
    assert!(md.contains("1-section"));
}

#[test]
fn truncate_stderr_caps_payload_with_suffix() {
    let raw = "y".repeat(2000);
    let out = DocumentError::truncate_stderr(&raw);
    assert!(out.chars().count() <= 500);
    assert!(out.ends_with("[…truncated]"));
    let short = "tiny error";
    assert_eq!(DocumentError::truncate_stderr(short), short);
}
