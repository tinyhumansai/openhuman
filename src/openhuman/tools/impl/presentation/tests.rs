//! Unit tests for the `generate_presentation` tool.
//!
//! Production end-to-end behaviour (real `python3` + `pip install python-pptx`)
//! is covered separately by `tests/presentation_tool.rs`, which is gated on the
//! host actually having Python available. The mock-driven cases here exist so
//! the validation / error-mapping branches stay covered on every CI machine,
//! regardless of Python availability.

use super::invoker::{InvocationOutcome, PythonInvoker};
use super::types::{MAX_BULLETS_PER_SLIDE, MAX_SLIDES, MAX_TEXT_CHARS};
use super::*;

use async_trait::async_trait;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Captures the last `run` call so tests can assert against the rendered argv
/// / stdin payload without going through a real Python interpreter.
#[derive(Default)]
struct MockPythonInvokerInner {
    last_script: Option<PathBuf>,
    last_stdin: Option<Vec<u8>>,
    last_output: Option<PathBuf>,
    last_deadline: Option<Duration>,
}

struct MockPythonInvoker {
    inner: Mutex<MockPythonInvokerInner>,
    outcome: Mutex<Option<InvocationOutcome>>,
    /// When set, the mock writes this byte payload to `output_path`
    /// before returning — emulates `python-pptx` having actually
    /// produced a file the tool can stat for size.
    output_bytes: Option<Vec<u8>>,
}

impl MockPythonInvoker {
    fn new(outcome: InvocationOutcome) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(MockPythonInvokerInner::default()),
            outcome: Mutex::new(Some(outcome)),
            output_bytes: None,
        })
    }

    fn new_writing(outcome: InvocationOutcome, payload: Vec<u8>) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(MockPythonInvokerInner::default()),
            outcome: Mutex::new(Some(outcome)),
            output_bytes: Some(payload),
        })
    }
}

#[async_trait]
impl PythonInvoker for MockPythonInvoker {
    async fn run(
        &self,
        script_path: &Path,
        stdin_payload: Vec<u8>,
        output_path: &Path,
        deadline: Duration,
    ) -> anyhow::Result<InvocationOutcome> {
        {
            let mut guard = self.inner.lock().unwrap();
            guard.last_script = Some(script_path.to_path_buf());
            guard.last_stdin = Some(stdin_payload);
            guard.last_output = Some(output_path.to_path_buf());
            guard.last_deadline = Some(deadline);
        }
        if let Some(payload) = self.output_bytes.as_ref() {
            tokio::fs::write(output_path, payload).await?;
        }
        let outcome = self
            .outcome
            .lock()
            .unwrap()
            .take()
            .expect("mock invoker called more than once");
        Ok(outcome)
    }
}

fn workspace() -> tempfile::TempDir {
    tempfile::tempdir().expect("create temp workspace")
}

fn minimal_input_json() -> serde_json::Value {
    json!({
        "title": "Test Deck",
        "slides": [
            { "title": "Intro", "body": "hello" }
        ]
    })
}

#[test]
fn parameters_schema_shape_matches_contract() {
    let tool = PresentationTool::with_invoker(
        PathBuf::from("/tmp/never-read"),
        MockPythonInvoker::new(InvocationOutcome::Success {
            stdout: String::new(),
            stderr: String::new(),
        }),
    );
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
    let tool = PresentationTool::with_invoker(
        PathBuf::from("/tmp/never-read"),
        MockPythonInvoker::new(InvocationOutcome::Success {
            stdout: String::new(),
            stderr: String::new(),
        }),
    );
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
}

#[test]
fn description_includes_router_rules() {
    let tool = PresentationTool::with_invoker(
        PathBuf::from("/tmp/never-read"),
        MockPythonInvoker::new(InvocationOutcome::Success {
            stdout: String::new(),
            stderr: String::new(),
        }),
    );
    let desc = tool.description();
    assert!(desc.contains("USE THIS"));
    assert!(desc.contains("NOT for"));
    assert!(desc.contains("slides") || desc.contains("deck") || desc.contains("presentation"));
}

#[tokio::test]
async fn execute_rejects_empty_title() {
    let ws = workspace();
    let tool = PresentationTool::with_invoker(
        ws.path().to_path_buf(),
        MockPythonInvoker::new(InvocationOutcome::Success {
            stdout: String::new(),
            stderr: String::new(),
        }),
    );
    let result = tool
        .execute(json!({ "title": "  ", "slides": [{ "title": "x" }] }))
        .await
        .expect("execute returns Ok with is_error=true");
    assert!(result.is_error);
    assert!(result.text().contains("title"));
}

#[tokio::test]
async fn execute_rejects_empty_slides_array() {
    let ws = workspace();
    let tool = PresentationTool::with_invoker(
        ws.path().to_path_buf(),
        MockPythonInvoker::new(InvocationOutcome::Success {
            stdout: String::new(),
            stderr: String::new(),
        }),
    );
    let result = tool
        .execute(json!({ "title": "Deck", "slides": [] }))
        .await
        .expect("execute returns Ok");
    assert!(result.is_error);
    assert!(result.text().contains("slides"));
}

#[tokio::test]
async fn execute_rejects_slide_with_no_content() {
    let ws = workspace();
    let tool = PresentationTool::with_invoker(
        ws.path().to_path_buf(),
        MockPythonInvoker::new(InvocationOutcome::Success {
            stdout: String::new(),
            stderr: String::new(),
        }),
    );
    let result = tool
        .execute(json!({
            "title": "Deck",
            "slides": [{ "title": "", "body": "", "bullets": [] }],
        }))
        .await
        .expect("execute returns Ok");
    assert!(result.is_error);
    assert!(result.text().contains("title / body / bullets"));
}

#[tokio::test]
async fn execute_rejects_oversize_body() {
    let ws = workspace();
    let tool = PresentationTool::with_invoker(
        ws.path().to_path_buf(),
        MockPythonInvoker::new(InvocationOutcome::Success {
            stdout: String::new(),
            stderr: String::new(),
        }),
    );
    let huge = "x".repeat(MAX_TEXT_CHARS + 1);
    let result = tool
        .execute(json!({
            "title": "Deck",
            "slides": [{ "title": "t", "body": huge }],
        }))
        .await
        .expect("execute returns Ok");
    assert!(result.is_error);
    assert!(result.text().contains("body"));
}

#[tokio::test]
async fn execute_rejects_too_many_slides() {
    let ws = workspace();
    let tool = PresentationTool::with_invoker(
        ws.path().to_path_buf(),
        MockPythonInvoker::new(InvocationOutcome::Success {
            stdout: String::new(),
            stderr: String::new(),
        }),
    );
    let slides = (0..(MAX_SLIDES + 1))
        .map(|i| json!({ "title": format!("s{i}") }))
        .collect::<Vec<_>>();
    let result = tool
        .execute(json!({ "title": "Deck", "slides": slides }))
        .await
        .expect("execute returns Ok");
    assert!(result.is_error);
    assert!(result.text().contains("slides"));
}

#[tokio::test]
async fn execute_happy_path_returns_artifact_metadata() {
    let ws = workspace();
    // Produce a small payload so the finalize step sees a non-zero size.
    let bogus_pptx = b"PK\x03\x04mock-pptx-bytes".to_vec();
    let invoker = MockPythonInvoker::new_writing(
        InvocationOutcome::Success {
            stdout: r#"{"ok":true,"slide_count":1}"#.to_string(),
            stderr: String::new(),
        },
        bogus_pptx.clone(),
    );
    let tool = PresentationTool::with_invoker(ws.path().to_path_buf(), invoker);

    let result = tool
        .execute(minimal_input_json())
        .await
        .expect("execute returns Ok");
    assert!(!result.is_error, "expected success, got {}", result.text());

    // Parse the structured payload off the json content block.
    let json_val = result
        .content
        .iter()
        .find_map(|c| match c {
            crate::openhuman::skills::types::ToolContent::Json { data } => Some(data.clone()),
            _ => None,
        })
        .expect("expected a json content block");
    assert!(json_val["artifact_id"].is_string());
    assert_eq!(json_val["slide_count"], 1);
    assert_eq!(json_val["size_bytes"], bogus_pptx.len() as u64);

    // Artifact file should exist on disk under the workspace.
    let artifact_path = PathBuf::from(json_val["artifact_path"].as_str().unwrap());
    assert!(artifact_path.exists(), "artifact not at {artifact_path:?}");
    let written = tokio::fs::read(&artifact_path).await.unwrap();
    assert_eq!(written, bogus_pptx);
}

#[tokio::test]
async fn execute_surfaces_generation_failed_with_truncated_stderr() {
    let ws = workspace();
    let long_stderr = "x".repeat(2000); // beyond 500-char cap
    let invoker = MockPythonInvoker::new(InvocationOutcome::NonZeroExit {
        exit_code: 3,
        stdout: String::new(),
        stderr: long_stderr.clone(),
    });
    let tool = PresentationTool::with_invoker(ws.path().to_path_buf(), invoker);

    let result = tool
        .execute(minimal_input_json())
        .await
        .expect("execute returns Ok");
    assert!(result.is_error);
    assert!(result.text().contains("python-pptx generation failed"));
    assert!(result.text().contains("exit=3"));
    assert!(
        result.text().contains("[…truncated]"),
        "stderr should be truncated"
    );
    // The message length should be bounded by the truncation budget plus the
    // surrounding error format — well below the original 2000-char dump.
    assert!(result.text().len() < 1000);
}

#[tokio::test]
async fn execute_surfaces_generation_timeout() {
    let ws = workspace();
    let invoker = MockPythonInvoker::new(InvocationOutcome::Timeout { timeout_secs: 60 });
    let tool = PresentationTool::with_invoker(ws.path().to_path_buf(), invoker);

    let result = tool
        .execute(minimal_input_json())
        .await
        .expect("execute returns Ok");
    assert!(result.is_error);
    assert!(result.text().contains("exceeded 60s timeout"));
}

#[tokio::test]
async fn execute_surfaces_missing_runtime() {
    let ws = workspace();
    let invoker = MockPythonInvoker::new(InvocationOutcome::MissingRuntime {
        reason: "no python on PATH".to_string(),
    });
    let tool = PresentationTool::with_invoker(ws.path().to_path_buf(), invoker);

    let result = tool
        .execute(minimal_input_json())
        .await
        .expect("execute returns Ok");
    assert!(result.is_error);
    assert!(result.text().contains("python runtime is not available"));
    assert!(result.text().contains("no python on PATH"));
}

#[tokio::test]
async fn execute_surfaces_missing_package() {
    let ws = workspace();
    let invoker = MockPythonInvoker::new(InvocationOutcome::PackageInstallFailed {
        reason: "pip install failed (exit=1): network unreachable".to_string(),
    });
    let tool = PresentationTool::with_invoker(ws.path().to_path_buf(), invoker);

    let result = tool
        .execute(minimal_input_json())
        .await
        .expect("execute returns Ok");
    assert!(result.is_error);
    assert!(result.text().contains("python-pptx"));
    assert!(result.text().contains("first-call venv setup failed"));
}

#[tokio::test]
async fn execute_marks_artifact_failed_when_script_drops_file() {
    let ws = workspace();
    // Success outcome but mock does NOT write any file to output_path.
    let invoker = MockPythonInvoker::new(InvocationOutcome::Success {
        stdout: String::new(),
        stderr: String::new(),
    });
    let tool = PresentationTool::with_invoker(ws.path().to_path_buf(), invoker);

    let result = tool
        .execute(minimal_input_json())
        .await
        .expect("execute returns Ok");
    assert!(result.is_error);
    assert!(result.text().contains("no output file"));
}

#[test]
fn truncate_stderr_caps_payload_with_suffix() {
    let raw = "y".repeat(2000);
    let out = types::PresentationError::truncate_stderr(&raw);
    assert!(out.chars().count() <= 500);
    assert!(out.ends_with("[…truncated]"));
    let short = "tiny stderr";
    assert_eq!(types::PresentationError::truncate_stderr(short), short);
}
