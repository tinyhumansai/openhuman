//! End-to-end integration test for `generate_presentation` (#2778).
//!
//! Drives the real [`PresentationTool`] through a real Python
//! interpreter against a temp workspace, then asserts the produced
//! file is a valid `.pptx` (zip with `[Content_Types].xml`).
//!
//! Skipped on hosts that lack `python3` or `python-pptx` so contributors
//! without a Python install still see green locally. CI provisions the
//! dependency via `.github/workflows/coverage.yml`'s Rust-core lane.

use std::process::Command;
use std::sync::Arc;

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::tools::PresentationTool;
use openhuman_core::openhuman::tools::Tool;
use serde_json::json;

#[tokio::test]
async fn end_to_end_generates_real_pptx_when_python_pptx_available() {
    if !python_available() {
        eprintln!("skipping: no python3 on PATH");
        return;
    }
    if !python_pptx_importable() {
        eprintln!(
            "skipping: python3 cannot `import pptx` — install with `pip install python-pptx==1.0.2`"
        );
        return;
    }

    let ws = tempfile::tempdir().expect("workspace tempdir");
    let cache = tempfile::tempdir().expect("runtime_python cache tempdir");

    // Build a Config that points runtime_python at a temp cache + the
    // host's preferred Python, so the test does not reach for the
    // network / managed installer.
    let mut config = Config::default();
    config.runtime_python.enabled = true;
    config.runtime_python.prefer_system = true;
    config.runtime_python.cache_dir = cache.path().display().to_string();
    let config = Arc::new(config);

    let tool = PresentationTool::new(config, ws.path().to_path_buf());

    let input = json!({
        "title": "End-to-End Canary Deck",
        "author": "openhuman tests",
        "slides": [
            { "title": "Hello", "body": "first slide body" },
            {
                "title": "Bullets",
                "bullets": ["alpha", "beta", "gamma"],
                "speaker_notes": "talk slowly here",
            },
        ],
    });

    let result = tool.execute(input).await.expect("execute returns Ok");
    assert!(!result.is_error, "expected success, got: {}", result.text());

    let json_val = result
        .content
        .iter()
        .find_map(|c| match c {
            openhuman_core::openhuman::skills::types::ToolContent::Json { data } => {
                Some(data.clone())
            }
            _ => None,
        })
        .expect("expected a json content block");

    let path = json_val["artifact_path"]
        .as_str()
        .expect("artifact_path string");
    let bytes = std::fs::read(path).expect("read produced pptx");
    assert!(
        bytes.starts_with(b"PK\x03\x04"),
        "pptx should start with zip magic, got first bytes {:?}",
        &bytes[..bytes.len().min(8)]
    );
    // [Content_Types].xml is the OOXML manifest — its presence is a
    // cheap-but-meaningful sanity check that the zip is an actual
    // OOXML package (not just a random PK\x03\x04 blob).
    let content_types_present = bytes
        .windows(b"[Content_Types].xml".len())
        .any(|w| w == b"[Content_Types].xml");
    assert!(
        content_types_present,
        "expected [Content_Types].xml inside the pptx zip"
    );
}

#[tokio::test]
async fn end_to_end_rejects_invalid_input_without_spawning_python() {
    // No Python required — validation runs before invoker.run.
    let ws = tempfile::tempdir().expect("workspace tempdir");

    let mut config = Config::default();
    config.runtime_python.enabled = true;
    let config = Arc::new(config);
    let tool = PresentationTool::new(config, ws.path().to_path_buf());

    let result = tool
        .execute(json!({ "title": "", "slides": [{ "title": "x" }] }))
        .await
        .expect("execute returns Ok with is_error=true");
    assert!(result.is_error);
    assert!(result.text().contains("title"));
}

fn python_available() -> bool {
    for name in ["python3", "python"] {
        if let Ok(output) = Command::new(name).arg("--version").output() {
            if output.status.success() {
                return true;
            }
        }
    }
    false
}

fn python_pptx_importable() -> bool {
    for name in ["python3", "python"] {
        if let Ok(output) = Command::new(name).arg("-c").arg("import pptx").output() {
            if output.status.success() {
                return true;
            }
        }
    }
    false
}
