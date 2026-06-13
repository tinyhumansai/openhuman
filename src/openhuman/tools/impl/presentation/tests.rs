//! Unit tests for the `generate_presentation` tool.
//!
//! The engine layer (`engine.rs`) ships its own focused tests covering
//! the `SlideSpec` → `ppt-rs` mapping, OOXML round-trip, and timeout
//! handling. The tests here cover the tool-level concerns: input
//! validation rejection branches, the parameters schema contract, the
//! `description` router rules, the artifact-pipeline glue, and the
//! happy-path output shape (artifact id + path + slide count + size).
//!
//! No mocks or interpreters — the real engine runs every test, so the
//! happy-path assertion doubles as a contract check that the engine
//! swap continues to produce a valid `.pptx` from this tool's
//! perspective.

use super::types::{PresentationError, MAX_BULLETS_PER_SLIDE, MAX_SLIDES, MAX_TEXT_CHARS};
use super::*;

use std::path::Path;

fn workspace() -> tempfile::TempDir {
    tempfile::tempdir().expect("create temp workspace")
}

/// A permissive policy rooted at `workspace` so File-source images written
/// under the temp workspace pass `validate_path`. Mirrors the pattern used
/// by the browser tool tests (`image_info.rs`).
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

/// Build a tool whose security policy is rooted at `workspace`.
fn make_tool(workspace: &Path) -> PresentationTool {
    PresentationTool::new(workspace.to_path_buf(), test_security(workspace))
}

fn minimal_input_json() -> serde_json::Value {
    json!({
        "title": "Quarterly Review",
        "slides": [
            { "title": "Highlights", "bullets": ["Up and to the right"] }
        ]
    })
}

#[test]
fn parameters_schema_shape_matches_contract() {
    let tool = make_tool(Path::new("/tmp/never-read"));
    let schema = tool.parameters_schema();
    assert_eq!(schema["type"], "object");
    let required = schema["required"].as_array().expect("required is array");
    assert!(required.iter().any(|v| v.as_str() == Some("title")));
    assert!(required.iter().any(|v| v.as_str() == Some("slides")));
    assert_eq!(schema["additionalProperties"], false);
    let title_props = &schema["properties"]["title"];
    assert_eq!(title_props["type"], "string");
    assert_eq!(title_props["maxLength"], MAX_TEXT_CHARS);
    let slides = &schema["properties"]["slides"];
    assert_eq!(slides["minItems"], 1);
    assert_eq!(slides["maxItems"], MAX_SLIDES);
    let slide_item = &slides["items"];
    assert_eq!(slide_item["additionalProperties"], false);
    let bullets = &slide_item["properties"]["bullets"];
    assert_eq!(bullets["maxItems"], MAX_BULLETS_PER_SLIDE);
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
    assert!(desc.contains("slides") || desc.contains("deck") || desc.contains("presentation"));
}

#[tokio::test]
async fn execute_rejects_empty_title() {
    let ws = workspace();
    let tool = make_tool(ws.path());
    let args = json!({ "title": "", "slides": [{ "title": "x", "bullets": ["y"] }] });
    let result = tool.execute(args).await.expect("execute returns Ok");
    assert!(result.is_error);
    assert!(result.text().contains("title"));
}

#[tokio::test]
async fn execute_rejects_empty_slides_array() {
    let ws = workspace();
    let tool = make_tool(ws.path());
    let args = json!({ "title": "Deck", "slides": [] });
    let result = tool.execute(args).await.expect("execute returns Ok");
    assert!(result.is_error);
    assert!(result.text().contains("slides"));
}

#[tokio::test]
async fn execute_rejects_slide_with_no_content() {
    let ws = workspace();
    let tool = make_tool(ws.path());
    let args = json!({
        "title": "Deck",
        "slides": [{ "title": "", "body": "", "bullets": [], "speaker_notes": "" }]
    });
    let result = tool.execute(args).await.expect("execute returns Ok");
    assert!(result.is_error);
}

#[tokio::test]
async fn execute_rejects_oversize_body() {
    let ws = workspace();
    let tool = make_tool(ws.path());
    let big = "x".repeat(MAX_TEXT_CHARS + 1);
    let args = json!({
        "title": "Deck",
        "slides": [{ "title": "ok", "body": big }]
    });
    let result = tool.execute(args).await.expect("execute returns Ok");
    assert!(result.is_error);
}

#[tokio::test]
async fn execute_rejects_too_many_slides() {
    let ws = workspace();
    let tool = make_tool(ws.path());
    let slides: Vec<_> = (0..(MAX_SLIDES + 1))
        .map(|i| json!({ "title": format!("Slide {i}"), "bullets": ["x"] }))
        .collect();
    let args = json!({ "title": "Big deck", "slides": slides });
    let result = tool.execute(args).await.expect("execute returns Ok");
    assert!(result.is_error);
    assert!(result.text().contains(&MAX_SLIDES.to_string()));
}

#[tokio::test]
async fn execute_happy_path_returns_artifact_metadata() {
    // End-to-end: drives the real ppt-rs engine and the artifact
    // pipeline. Asserts the tool's success contract — `slide_count`
    // excludes the synthetic title slide, the artifact is finalised
    // on disk, and the markdown reply quotes the path + size.
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

    let payload = match result.content.first().expect("at least one content block") {
        crate::openhuman::workflows::types::ToolContent::Json { data } => data.clone(),
        other => panic!("expected Json content block, got {other:?}"),
    };
    assert_eq!(payload["slide_count"].as_u64(), Some(1));
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
        size_bytes > 1000,
        "deck unexpectedly small ({size_bytes} bytes)"
    );

    let md = result
        .markdown_formatted
        .as_deref()
        .expect("success_with_markdown sets markdown_formatted");
    assert!(md.contains(artifact_id));
    assert!(md.contains(artifact_path));
    assert!(md.contains("1-slide"));
}

/// Canonical 1×1 PNG, written to disk to exercise the File source path.
fn png_1x1() -> Vec<u8> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==")
        .unwrap()
}

/// Pull the JSON payload out of a tool result.
fn payload_of(result: &ToolResult) -> serde_json::Value {
    match result.content.first().expect("a content block") {
        crate::openhuman::workflows::types::ToolContent::Json { data } => data.clone(),
        other => panic!("expected Json content block, got {other:?}"),
    }
}

/// Read the entry names of the produced .pptx artifact.
fn pptx_entry_names(artifact_path: &str) -> Vec<String> {
    let bytes = std::fs::read(artifact_path).expect("artifact file readable");
    let cursor = std::io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(cursor).expect("artifact is a valid zip");
    (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_string())
        .collect()
}

#[tokio::test]
async fn execute_embeds_file_image_into_deck() {
    let ws = workspace();
    let img_path = ws.path().join("chart.png");
    std::fs::write(&img_path, png_1x1()).expect("write png");

    let tool = make_tool(ws.path());
    let args = json!({
        "title": "Deck with image",
        "slides": [{
            "title": "Chart",
            "bullets": ["See below"],
            "images": [{
                "source": { "type": "file", "path": img_path.to_string_lossy() },
                "caption": "Quarterly revenue"
            }]
        }]
    });
    let result = tool.execute(args).await.expect("execute returns Ok");
    assert!(!result.is_error, "valid image should not error the deck");

    let payload = payload_of(&result);
    assert!(
        payload["image_warnings"]
            .as_array()
            .map(|a| a.is_empty())
            .unwrap_or(true),
        "no warnings expected for a valid PNG: {:?}",
        payload["image_warnings"]
    );

    let names = pptx_entry_names(payload["artifact_path"].as_str().unwrap());
    assert!(
        names.iter().any(|n| n == "ppt/media/image1.png"),
        "embedded PNG missing from artifact (got: {names:?})"
    );
}

#[tokio::test]
async fn execute_skips_unsupported_mime_image_with_warning() {
    let ws = workspace();
    let txt_path = ws.path().join("notanimage.txt");
    std::fs::write(&txt_path, b"i am plain text, not an image").expect("write txt");

    let tool = make_tool(ws.path());
    let args = json!({
        "title": "Deck",
        "slides": [{
            "title": "Slide",
            "bullets": ["text"],
            "images": [{ "source": { "type": "file", "path": txt_path.to_string_lossy() } }]
        }]
    });
    let result = tool.execute(args).await.expect("execute returns Ok");
    // Partial success: deck still produced, but the bad image is reported.
    assert!(!result.is_error, "bad image must not fail the whole deck");
    let payload = payload_of(&result);
    let warnings = payload["image_warnings"]
        .as_array()
        .expect("warnings array");
    assert_eq!(warnings.len(), 1, "exactly one image warning expected");
    assert!(
        warnings[0]
            .as_str()
            .unwrap()
            .contains("unsupported image type"),
        "warning should name the MIME problem: {:?}",
        warnings[0]
    );
}

#[tokio::test]
async fn execute_skips_oversize_image_with_warning() {
    let ws = workspace();
    let big_path = ws.path().join("huge.png");
    // 5 MB + 1 byte — exceeds the per-image cap. Caught at metadata stat
    // before the bytes are pulled into memory.
    std::fs::write(&big_path, vec![0u8; 5 * 1024 * 1024 + 1]).expect("write big file");

    let tool = make_tool(ws.path());
    let args = json!({
        "title": "Deck",
        "slides": [{
            "title": "Slide",
            "bullets": ["text"],
            "images": [{ "source": { "type": "file", "path": big_path.to_string_lossy() } }]
        }]
    });
    let result = tool.execute(args).await.expect("execute returns Ok");
    assert!(!result.is_error);
    let payload = payload_of(&result);
    let warnings = payload["image_warnings"]
        .as_array()
        .expect("warnings array");
    assert_eq!(warnings.len(), 1);
    assert!(
        warnings[0].as_str().unwrap().contains("cap"),
        "warning should mention the size cap: {:?}",
        warnings[0]
    );
}

#[tokio::test]
async fn execute_skips_missing_artifact_with_warning() {
    let ws = workspace();
    let tool = make_tool(ws.path());
    let args = json!({
        "title": "Deck",
        "slides": [{
            "title": "Slide",
            "bullets": ["text"],
            "images": [{ "source": { "type": "artifact", "artifact_id": "does-not-exist" } }]
        }]
    });
    let result = tool.execute(args).await.expect("execute returns Ok");
    assert!(!result.is_error);
    let payload = payload_of(&result);
    let warnings = payload["image_warnings"]
        .as_array()
        .expect("warnings array");
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].as_str().unwrap().contains("unreadable"));
}

#[tokio::test]
async fn execute_rejects_too_many_images_per_deck() {
    let ws = workspace();
    let tool = make_tool(ws.path());
    // 9 images total across slides — exceeds the deck cap of 8. This is a
    // hard validation reject (cheap structural check), not a skip.
    let images: Vec<_> = (0..9)
        .map(|i| json!({ "source": { "type": "artifact", "artifact_id": format!("a{i}") } }))
        .collect();
    let args = json!({
        "title": "Deck",
        "slides": [{ "title": "Slide", "bullets": ["x"], "images": images }]
    });
    let result = tool.execute(args).await.expect("execute returns Ok");
    assert!(result.is_error, "9 images should be rejected outright");
    assert!(result.text().contains("images"));
}

#[test]
fn truncate_stderr_caps_payload_with_suffix() {
    let raw = "y".repeat(2000);
    let out = PresentationError::truncate_stderr(&raw);
    assert!(out.chars().count() <= 500);
    assert!(out.ends_with("[…truncated]"));
    let short = "tiny stderr";
    assert_eq!(PresentationError::truncate_stderr(short), short);
}

#[test]
fn unsupported_file_type_display_includes_extension_and_supported() {
    // Reserved-for-future variant (#2780): confirms the Display impl
    // renders both the rejected extension and the supported set so a
    // downstream mapper can surface a user-correctable message verbatim
    // once a `format` selector lands.
    let err = PresentationError::UnsupportedFileType {
        extension: "key".to_string(),
        supported: "pptx".to_string(),
    };
    let rendered = err.to_string();
    assert!(rendered.contains("unsupported file type"));
    assert!(rendered.contains("'key'"));
    assert!(rendered.contains("supported: pptx"));
}
