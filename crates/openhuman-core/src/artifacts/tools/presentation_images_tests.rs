//! Unit tests for slide-image resolution (`presentation_images`): skip-with-
//! warning on unsupported, oversize, and missing images, and no host fallback
//! when the image inspector's module is disabled.

use super::*;
use crate::artifacts::tools::presentation::PresentationTool;
use crate::artifacts::tools::presentation_types::validate_image_references;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;

fn workspace() -> tempfile::TempDir {
    tempfile::tempdir().expect("create temp workspace")
}

/// A permissive policy rooted at `workspace` so File-source images written
/// under the temp workspace pass `validate_path`.
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
    PresentationTool::with_config(
        workspace.to_path_buf(),
        workspace.join("Files"),
        test_security(workspace),
        config,
    )
}

/// Canonical 1×1 PNG, written to disk to exercise the File source path.
fn png_1x1() -> Vec<u8> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==")
        .unwrap()
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
        .image_sources()
        .resolve_with_inspector(&input, &UnsupportedImage)
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

    let (resolved, warnings) = tool
        .image_sources()
        .resolve(&input, &DispatchCtx::ambient())
        .await;
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
    let (resolved, warnings) = tool
        .image_sources()
        .resolve(&input, &DispatchCtx::ambient())
        .await;
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
    let (resolved, warnings) = tool
        .image_sources()
        .resolve(&input, &DispatchCtx::ambient())
        .await;
    assert!(
        resolved.iter().all(Vec::is_empty),
        "a rejected image must not reach the deck"
    );
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("unreadable"));
}
