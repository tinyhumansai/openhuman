//! End-to-end integration test for `generate_presentation` (#2778).
//!
//! Drives the real [`PresentationTool`] through a real Python
//! interpreter against a temp workspace, then asserts the produced
//! file is a valid `.pptx` (zip with `[Content_Types].xml`).
//!
//! Skipped on hosts whose `PATH` carries no Python ≥ `3.12.0` (the
//! `runtime_python` floor) so contributors without a compatible
//! install still see green locally. CI provisions a 3.12 interpreter
//! via `.github/workflows/coverage.yml`'s Rust-core lane; pip and
//! `python-pptx` are installed by `runtime_python::venv::ensure_venv`
//! inside the test's tempdir cache (the venv has no host
//! `site-packages`), so the host does not need `python-pptx`
//! pre-installed.

use std::sync::Arc;

use openhuman_core::openhuman::config::schema::RuntimePythonConfig;
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::runtime_python::detect_system_python;
use openhuman_core::openhuman::tools::PresentationTool;
use openhuman_core::openhuman::tools::Tool;
use serde_json::json;

#[tokio::test]
async fn end_to_end_generates_real_pptx_when_python_pptx_available() {
    // Mirror runtime_python::bootstrap's interpreter selection so the
    // skip gate and the in-test invoker land on the same binary. The
    // free-form `resolved_python()` + `python -c "import pptx"` gate
    // we used to run was misaligned with the runtime two ways:
    //
    //   1. Candidate order — runtime probes `python3.12` → `python3`
    //      → `python` and rejects anything below `minimum_version`
    //      (default 3.12.0). A free `python3`-first probe would pass
    //      on a 3.10 host that the runtime would then refuse, leaving
    //      the test to skip on the wrong axis.
    //   2. The `import pptx` probe gated on host site-packages, but
    //      `ensure_venv` creates an isolated venv (no
    //      `--system-site-packages`) and pip-installs
    //      `python-pptx==1.0.2` itself — host importability says
    //      nothing about whether the venv path will work.
    let defaults = RuntimePythonConfig::default();
    if detect_system_python(&defaults.minimum_version, None).is_none() {
        eprintln!(
            "skipping: no Python >= {} on PATH (runtime_python floor); \
             install python3.12+ to exercise this test",
            defaults.minimum_version
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
