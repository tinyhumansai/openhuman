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

use super::types::GeneratePresentationInput;
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
    use crate::security::AutonomyLevel;
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
    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.to_path_buf();
    config.modules.enabled = false;
    make_tool_with_config(workspace, config)
}

fn make_tool_with_config(workspace: &Path, config: crate::config::Config) -> PresentationTool {
    PresentationTool::with_config(
        workspace.to_path_buf(),
        workspace.join("Files"),
        test_security(workspace),
        config,
    )
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
async fn execute_rejects_too_many_slides() {
    let ws = workspace();
    let tool = make_tool(ws.path());
    let slides: Vec<_> = (0..(MAX_SLIDES + 1))
        .map(|i| json!({ "title": format!("Slide {i}"), "bullets": ["x"] }))
        .collect();
    let args = json!({ "title": "Big deck", "slides": slides });
    let result = tool.execute(args).await.expect("execute returns Ok");
    assert!(result.is_error);
    assert!(
        result.text().contains("must contain ≤ 64 slides"),
        "the tool should preserve the bus limit in its error: {}",
        result.text()
    );
}

async fn assert_rejected_before_image_resolution(args: serde_json::Value, expected: &str) {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingResolver(AtomicUsize);
    #[async_trait]
    impl ImageResolver for CountingResolver {
        async fn resolve(
            &self,
            _tool: &PresentationTool,
            input: &GeneratePresentationInput,
            _ctx: &DispatchCtx,
        ) -> (Vec<Vec<SlideImage>>, Vec<String>) {
            self.0.fetch_add(1, Ordering::SeqCst);
            (vec![Vec::new(); input.slides.len()], Vec::new())
        }
    }

    let ws = workspace();
    let tool = make_tool(ws.path());
    let resolver = CountingResolver(AtomicUsize::new(0));
    let result = tool
        .run_with_resolver(args, None, &DispatchCtx::ambient(), &resolver)
        .await
        .expect("run returns tool result");

    assert!(
        result.is_error,
        "invalid request should fail: {}",
        result.text()
    );
    let resolution_calls = resolver.0.load(Ordering::SeqCst);
    let files_created = ws.path().join("Files").exists();
    assert!(
        resolution_calls == 0 && !files_created,
        "expected zero image resolution calls and no artifact output; got {resolution_calls} calls, Files created={files_created}"
    );
    assert!(result.text().contains(expected), "{}", result.text());
}

#[tokio::test]
async fn request_preflight_rejects_invalid_text_before_image_resolution() {
    let args = json!({
        "title": " ",
        "slides": [{
            "title": "Slide",
            "bullets": ["valid"],
            "images": [{ "source": { "type": "file", "path": "missing.png" } }]
        }]
    });
    assert_rejected_before_image_resolution(args, "title").await;
}

#[tokio::test]
async fn request_preflight_rejects_too_many_slides_before_image_resolution() {
    let args = json!({
        "title": "Deck",
        "slides": (0..=MAX_SLIDES)
            .map(|i| json!({ "title": format!("Slide {i}"), "bullets": ["valid"] }))
            .collect::<Vec<_>>()
    });
    assert_rejected_before_image_resolution(args, "64 slides").await;
}

#[tokio::test]
async fn request_preflight_counts_failed_image_references_before_resolution() {
    let args = json!({
        "title": "Deck",
        "slides": [{
            "title": "Slide",
            "bullets": ["valid"],
            "images": (0..9).map(|i| json!({
                "source": { "type": "file", "path": format!("missing-{i}.png") },
                "caption": "requested caption"
            })).collect::<Vec<_>>()
        }]
    });
    assert_rejected_before_image_resolution(args, "8 images total").await;
}

#[tokio::test]
async fn request_preflight_checks_failed_reference_captions_before_resolution() {
    let args = json!({
        "title": "Deck",
        "slides": [{
            "title": "Slide",
            "bullets": ["valid"],
            "images": [{
                "source": { "type": "file", "path": "missing.png" },
                "caption": "x".repeat(MAX_TEXT_CHARS + 1)
            }]
        }]
    });
    assert_rejected_before_image_resolution(args, "caption").await;
}

#[test]
fn resolved_presentation_uses_bus_validation_and_preserves_its_limit_error() {
    let input = GeneratePresentationInput {
        title: "Deck".to_string(),
        artifact_id: None,
        author: None,
        theme: None,
        slides: (0..=MAX_SLIDES)
            .map(|i| super::types::SlideRequest {
                title: format!("Slide {i}"),
                body: None,
                bullets: Vec::new(),
                speaker_notes: None,
                images: Vec::new(),
            })
            .collect(),
    };
    let images = vec![Vec::new(); input.slides.len()];
    let spec = super::types::to_bus_spec(&input, images).expect("one image list per slide");

    let error = spec
        .validate()
        .expect_err("the bus slide limit is enforced");
    let error = PresentationError::from(error);
    match error {
        PresentationError::InvalidInput { field, reason } => {
            assert_eq!(field, "slides");
            assert_eq!(reason, "must contain ≤ 64 slides");
        }
        other => panic!("expected bus validation to map to InvalidInput, got {other:?}"),
    }
}

#[tokio::test]
async fn execute_happy_path_returns_artifact_metadata() {
    if crate::util::module_test_process::run(
        module_path!(),
        "execute_happy_path_returns_artifact_metadata",
    ) {
        return;
    }
    // End-to-end: drives the real ppt-rs engine and the artifact
    // pipeline. Asserts the tool's success contract — `slide_count`
    // excludes the synthetic title slide, the artifact is finalised
    // on disk, and the markdown reply quotes the path + size.
    let ws = workspace();
    let mut config = crate::config::Config::default();
    config.workspace_dir = ws.path().to_path_buf();
    config.modules.allow_download = false;
    let tool = make_tool_with_config(ws.path(), config);
    let result = tool
        .execute(minimal_input_json())
        .await
        .expect("execute returns Ok");

    // `text()` is empty for a Json-only result, so it would report nothing on
    // failure. Render the blocks themselves.
    assert!(
        !result.is_error,
        "happy path should not be flagged as error: {:?}",
        result.content
    );

    let payload = match result.content.first().expect("at least one content block") {
        tinytools::ToolContent::Json { data } => data.clone(),
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
        tinytools::ToolContent::Json { data } => data.clone(),
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
    if crate::util::module_test_process::run(module_path!(), "execute_embeds_file_image_into_deck")
    {
        return;
    }
    let ws = workspace();
    let img_path = ws.path().join("chart.png");
    std::fs::write(&img_path, png_1x1()).expect("write png");

    let mut config = crate::config::Config::default();
    config.workspace_dir = ws.path().to_path_buf();
    config.modules.allow_download = false;
    let tool = make_tool_with_config(ws.path(), config);
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
    struct UnsupportedImage;
    #[async_trait]
    impl ImageInspector for UnsupportedImage {
        async fn inspect(&self, bytes: &[u8]) -> Result<tinydocs_bus::ImageFacts, String> {
            assert_eq!(bytes, b"i am plain text, not an image");
            Err("unsupported image type (only PNG and JPEG are embeddable)".into())
        }
    }
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
    let input: GeneratePresentationInput = serde_json::from_value(args).expect("args parse");
    let (resolved, warnings) = tool
        .resolve_images_with_inspector(&input, &UnsupportedImage)
        .await;
    // Partial success: the deck still renders, the bad image is reported.
    assert!(
        resolved.iter().all(Vec::is_empty),
        "a rejected image must not reach the deck"
    );
    assert_eq!(warnings.len(), 1, "exactly one image warning expected");
    assert!(
        !warnings[0].is_empty(),
        "the image rejection should be explained: {:?}",
        warnings[0]
    );
}

#[tokio::test]
async fn image_inspection_does_not_fall_back_when_modules_are_disabled() {
    let ws = workspace();
    let img_path = ws.path().join("chart.png");
    std::fs::write(&img_path, png_1x1()).expect("write png");
    let mut config = crate::config::Config::default();
    config.workspace_dir = ws.path().to_path_buf();
    config.modules.enabled = false;
    let tool = PresentationTool::with_config(
        ws.path().to_path_buf(),
        ws.path().join("Files"),
        test_security(ws.path()),
        config,
    );
    let input: GeneratePresentationInput = serde_json::from_value(json!({
        "title": "No host image parser",
        "slides": [{
            "title": "Chart",
            "bullets": ["See below"],
            "images": [{ "source": { "type": "file", "path": img_path.to_string_lossy() } }]
        }]
    }))
    .expect("parse presentation input");
    validate_image_references(&input).expect("valid host image reference");

    let (resolved, warnings) = tool.resolve_images(&input, &DispatchCtx::ambient()).await;
    assert!(resolved.iter().all(Vec::is_empty));
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("disabled"), "{warnings:?}");
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
    let input: GeneratePresentationInput = serde_json::from_value(args).expect("args parse");
    let (resolved, warnings) = tool.resolve_images(&input, &DispatchCtx::ambient()).await;
    assert!(
        resolved.iter().all(Vec::is_empty),
        "a rejected image must not reach the deck"
    );
    assert_eq!(warnings.len(), 1);
    assert!(
        warnings[0].contains("cap"),
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
    let input: GeneratePresentationInput = serde_json::from_value(args).expect("args parse");
    let (resolved, warnings) = tool.resolve_images(&input, &DispatchCtx::ambient()).await;
    assert!(
        resolved.iter().all(Vec::is_empty),
        "a rejected image must not reach the deck"
    );
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("unreadable"));
}

#[tokio::test]
async fn execute_rejects_too_many_images_per_deck() {
    if crate::util::module_test_process::run(
        module_path!(),
        "execute_rejects_too_many_images_per_deck",
    ) {
        return;
    }
    let ws = workspace();
    let mut config = crate::config::Config::default();
    config.workspace_dir = ws.path().to_path_buf();
    config.modules.allow_download = false;
    let tool = make_tool_with_config(ws.path(), config);
    let image_path = ws.path().join("one.png");
    std::fs::write(&image_path, png_1x1()).expect("write valid image");
    // Nine successfully resolved images exceed the bus deck cap of eight.
    let images: Vec<_> = (0..9)
        .map(|_| json!({ "source": { "type": "file", "path": image_path.to_string_lossy() } }))
        .collect();
    let args = json!({
        "title": "Deck",
        "slides": [{ "title": "Slide", "bullets": ["x"], "images": images }]
    });
    let result = tool.execute(args).await.expect("execute returns Ok");
    assert!(
        result.is_error,
        "9 resolved images should exceed the bus limit"
    );
    assert!(
        result.text().contains("must contain ≤ 8 images total"),
        "the tool should preserve the bus limit in its error: {}",
        result.text()
    );
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
