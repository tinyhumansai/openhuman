use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use tinyagents_harness::media::MediaOutput;
use tinystoragedrivers::BlobStore;
use tinytools::{PermissionLevel, Tool, ToolCategory, ToolContent, ToolResult};

use super::{copy_blob_to_file, has_reusable_files_reference, MediaArtifactTool};
use crate::agent::artifacts::{ArtifactKind, ArtifactStatus, FileRoots};

#[test]
fn artifact_history_is_refreshed_only_for_absolute_paths_in_files_roots() {
    let root = tempfile::tempdir().unwrap();
    let files = root.path().join("Files");
    std::fs::create_dir_all(&files).unwrap();
    let roots = FileRoots::new(&files);
    let filed = files.join("generated.png");
    let outside = root.path().join("action/generated.png");

    assert!(has_reusable_files_reference(
        &json!({ "references": [filed.to_string_lossy()] }),
        &roots
    ));
    assert!(!has_reusable_files_reference(
        &json!({ "references": ["https://example.test/image.png"] }),
        &roots
    ));
    assert!(!has_reusable_files_reference(
        &json!({ "first_frame": outside.to_string_lossy() }),
        &roots
    ));
    assert!(!has_reusable_files_reference(
        &json!({ "references": ["generated.png"] }),
        &roots
    ));
}

#[tokio::test]
async fn artifact_history_refresh_failure_fails_closed_for_cached_files() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("not-a-workspace-directory");
    std::fs::write(&workspace, b"force artifact listing failure").unwrap();
    let action = root.path().join("action");
    let files = root.path().join("Files");
    std::fs::create_dir_all(&action).unwrap();
    std::fs::create_dir_all(&files).unwrap();
    let replacement = files.join("generated.png");
    std::fs::write(&replacement, b"unrelated replacement").unwrap();
    let allowed = Arc::new(std::sync::Mutex::new(std::collections::HashSet::from([
        replacement.clone(),
    ])));
    let tool = MediaArtifactTool::new(
        StubMediaTool {
            file_count: 0,
            fail: false,
        },
        ArtifactKind::Image,
        workspace.clone(),
        files,
    )
    .with_reference_allowlist(Arc::clone(&allowed));

    assert!(allowed.lock().unwrap().contains(&replacement));
    assert!(tool
        .execute(json!({ "references": [replacement.to_string_lossy()] }))
        .await
        .is_err());
}

struct PartialThenFail(bool);

impl tokio::io::AsyncRead for PartialThenFail {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.0 {
            return std::task::Poll::Ready(Err(std::io::Error::other("reader failed")));
        }
        this.0 = true;
        buf.put_slice(b"partial media");
        std::task::Poll::Ready(Ok(()))
    }
}

/// A minimal stand-in for `GenerateImageTool` / `GenerateVideoTool`: writes
/// a fixed number of files under `<action_dir>/generated-media/` and reports
/// them in the same `artifacts` array shape the real vendor tools use.
struct StubMediaTool {
    file_count: usize,
    fail: bool,
}

#[async_trait]
impl Tool for StubMediaTool {
    fn name(&self) -> &str {
        "stub_generate"
    }

    fn description(&self) -> &str {
        "stub"
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object" })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        if self.fail {
            return Ok(ToolResult::error("billed and failed"));
        }
        let dir = args
            .get("__dir")
            .and_then(Value::as_str)
            .map(std::path::PathBuf::from)
            .expect("stub requires __dir");
        std::fs::create_dir_all(&dir).unwrap();
        let mut artifacts = Vec::new();
        for i in 0..self.file_count {
            let path = dir.join(format!("stub-{i}.png"));
            std::fs::write(&path, format!("bytes-{i}")).unwrap();
            artifacts.push(json!({
                "type": "image",
                "path": path.display().to_string(),
                "media_type": "image/png",
                "bytes": 8,
            }));
        }
        Ok(ToolResult::success_with_markdown(
            json!({ "model": "stub/model", "artifacts": artifacts }),
            "generated stub media",
        ))
    }
}

/// TinyAgents' production media output is a blob key, not a filesystem path.
struct BlobBackedMediaTool {
    blobs: Arc<dyn BlobStore>,
}

#[async_trait]
impl Tool for BlobBackedMediaTool {
    fn name(&self) -> &str {
        "blob_generate"
    }

    fn description(&self) -> &str {
        "blob-backed media stub"
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object" })
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        let key = "generated-media/stub-0.png";
        self.blobs
            .put(key, b"blob-media-bytes".to_vec(), Some("image/png"))
            .await?;
        Ok(ToolResult::success_with_markdown(
            json!({
                "artifacts": [{
                    "type": "image",
                    "path": key,
                    "media_type": "image/png",
                    "bytes": 16
                }]
            }),
            "Saved image at generated-media/stub-0.png\nCost: $0.01",
        ))
    }
}

fn json_payload(result: &ToolResult) -> Value {
    result
        .content
        .iter()
        .find_map(|block| match block {
            ToolContent::Json { data } => Some(data.clone()),
            _ => None,
        })
        .expect("json content block")
}

#[tokio::test]
async fn files_a_blob_backed_generated_artifact() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let files_dir = root.path().join("Files");
    let blobs = MediaOutput::new(root.path()).blobs.clone();
    let wrapped = MediaArtifactTool::new(
        BlobBackedMediaTool {
            blobs: Arc::clone(&blobs),
        },
        ArtifactKind::Image,
        workspace.clone(),
        files_dir.clone(),
    )
    .with_blobs(Arc::clone(&blobs));

    let result = wrapped.execute(json!({})).await.unwrap();
    assert!(!result.is_error, "{result:?}");
    let payload = json_payload(&result);
    let artifact = &payload["artifacts"][0];
    let artifact_id = artifact["artifact_id"]
        .as_str()
        .unwrap_or_else(|| panic!("blob-backed media was not filed: {artifact:?}"));
    let record = crate::agent::artifacts::store::get_artifact(&workspace, artifact_id)
        .await
        .expect("artifact record");
    let file = record.file.expect("visible file");
    let canonical_file = std::path::Path::new(&file).canonicalize().unwrap();
    assert_eq!(
        artifact["path"].as_str(),
        Some(canonical_file.to_str().unwrap())
    );
    assert_eq!(
        result.markdown_formatted.as_deref(),
        Some(format!("Saved image at {}\nCost: $0.01", canonical_file.display()).as_str())
    );
    assert_eq!(std::fs::read(&file).unwrap(), b"blob-media-bytes");
    assert!(blobs
        .get("generated-media/stub-0.png")
        .await
        .unwrap()
        .is_none());

    let (_, failed_file) = crate::agent::artifacts::create_artifact_for_call(
        &workspace,
        files_dir.clone(),
        ArtifactKind::Video,
        "Failed generated clip",
        "mp4",
        None,
        None,
    )
    .await
    .unwrap();
    std::fs::write(&failed_file, b"failed output").unwrap();
    let failed = crate::agent::artifacts::store::list_artifacts(&workspace, 0, 10, None)
        .await
        .unwrap()
        .0
        .into_iter()
        .find(|meta| meta.title == "Failed generated clip")
        .expect("failed artifact record");
    crate::agent::artifacts::fail_artifact(
        &workspace,
        files_dir.clone(),
        &failed.id,
        "test finalization failure",
        None,
    )
    .await
    .unwrap();

    let fresh = MediaArtifactTool::new(
        StubMediaTool {
            file_count: 0,
            fail: false,
        },
        ArtifactKind::Image,
        workspace.clone(),
        files_dir,
    )
    .with_blobs(blobs)
    .with_reference_allowlist(Arc::new(std::sync::Mutex::new(Default::default())));
    fresh.refresh_allowed_references().await.unwrap();
    {
        let allowed = fresh.allowed_references.as_ref().unwrap().lock().unwrap();
        assert!(allowed.contains(&canonical_file));
        assert!(!allowed.contains(&failed_file.canonicalize().unwrap()));
    }

    crate::agent::artifacts::fail_artifact(
        &workspace,
        fresh.files_dir.clone(),
        artifact_id,
        "revoked for the refresh test",
        None,
    )
    .await
    .unwrap();
    assert!(
        canonical_file.exists(),
        "failed regeneration retains the previous non-empty file"
    );
    fresh.refresh_allowed_references().await.unwrap();
    assert!(!fresh
        .allowed_references
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .contains(&canonical_file));
}

#[tokio::test]
async fn removes_partial_destination_when_blob_read_fails() {
    let root = tempfile::tempdir().unwrap();
    let dest = root.path().join("partial.png");
    std::fs::write(&dest, b"reserved placeholder").unwrap();

    let error = copy_blob_to_file(
        &mut PartialThenFail(false),
        &dest,
        "generated-media/fail.png",
    )
    .await
    .unwrap_err();

    assert!(error.contains("failed to copy generated media blob"));
    assert!(!dest.exists());
}

#[tokio::test]
async fn keeps_source_blob_when_artifact_finalization_fails() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let blobs = MediaOutput::new(root.path()).blobs.clone();
    let key = "generated-media/finalize-failure.png";
    blobs
        .put(key, b"media bytes".to_vec(), Some("image/png"))
        .await
        .unwrap();
    let tool = MediaArtifactTool::new(
        StubMediaTool {
            file_count: 0,
            fail: false,
        },
        ArtifactKind::Image,
        workspace,
        root.path().join("Files"),
    )
    .with_blobs(Arc::clone(&blobs));

    // An artifact whose file is gone cannot be completed.
    let production = crate::agent::artifacts::production::Production::start(
        &tool.workspace_dir,
        tool.files_dir.clone(),
        tiny_artifacts::ArtifactKind::Image,
        "Finalize failure",
        "png",
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let _ = std::fs::remove_file(production.path());
    let error = tool
        .complete_and_discard_blob(&production, Path::new(key))
        .await
        .unwrap_err();

    assert!(error.contains("artifact"));
    assert!(blobs.get(key).await.unwrap().is_some());
}

#[tokio::test]
async fn files_a_single_generated_artifact() {
    let root = tempfile::tempdir().unwrap();
    let staging = root.path().join("staging");
    let workspace = root.path().join("workspace");

    let wrapped = MediaArtifactTool::new(
        StubMediaTool {
            file_count: 1,
            fail: false,
        },
        ArtifactKind::Image,
        workspace.clone(),
        root.path().join("Files"),
    );

    let result = wrapped
        .execute(json!({ "prompt": "a cat", "__dir": staging.display().to_string() }))
        .await
        .unwrap();
    assert!(!result.is_error, "{result:?}");

    let payload = json_payload(&result);
    let artifacts = payload["artifacts"].as_array().unwrap();
    assert_eq!(artifacts.len(), 1);
    let artifact_id = artifacts[0]["artifact_id"].as_str().unwrap();
    assert!(artifacts[0].get("artifact_error").is_none());

    // Original staged file is gone (moved).
    assert!(!staging.join("stub-0.png").exists());
    // The artifact record lives in the document store (no `meta.json`); resolve
    // it through the store and verify its user-facing Files-folder path.
    let meta = crate::agent::artifacts::store::get_artifact(&workspace, artifact_id)
        .await
        .expect("artifact metadata is persisted");
    assert_eq!(meta.kind, ArtifactKind::Image);
    assert_eq!(meta.status, ArtifactStatus::Ready);
    let file = meta.file.expect("artifact points at its visible file");
    assert!(std::path::Path::new(&file).exists(), "{file}");
    assert_eq!(std::fs::read(file).unwrap(), b"bytes-0");
}

#[tokio::test]
async fn files_every_artifact_when_n_greater_than_one() {
    let root = tempfile::tempdir().unwrap();
    let staging = root.path().join("staging");
    let workspace = root.path().join("workspace");

    let wrapped = MediaArtifactTool::new(
        StubMediaTool {
            file_count: 3,
            fail: false,
        },
        ArtifactKind::Image,
        workspace.clone(),
        root.path().join("Files"),
    );

    let result = wrapped
        .execute(json!({ "prompt": "three cats", "__dir": staging.display().to_string() }))
        .await
        .unwrap();
    let payload = json_payload(&result);
    let artifacts = payload["artifacts"].as_array().unwrap();
    assert_eq!(artifacts.len(), 3);
    let mut ids = std::collections::HashSet::new();
    for entry in artifacts {
        let id = entry["artifact_id"].as_str().expect("artifact_id set");
        assert!(ids.insert(id.to_string()), "artifact ids must be unique");
        let meta = crate::agent::artifacts::store::get_artifact(&workspace, id)
            .await
            .expect("artifact metadata is persisted");
        assert_eq!(meta.status, ArtifactStatus::Ready);
        assert!(meta
            .file
            .as_deref()
            .is_some_and(|file| std::path::Path::new(file).exists()));
    }
}

#[tokio::test]
async fn leaves_an_errored_tool_result_untouched() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");

    let wrapped = MediaArtifactTool::new(
        StubMediaTool {
            file_count: 1,
            fail: true,
        },
        ArtifactKind::Image,
        workspace.clone(),
        root.path().join("Files"),
    );

    let result = wrapped.execute(json!({ "prompt": "x" })).await.unwrap();
    assert!(result.is_error);
    // No artifacts root should even be created.
    assert!(!workspace.join("artifacts").exists());
}

#[tokio::test]
async fn host_metadata_forwards_to_the_inner_tool() {
    let workspace_dir = tempfile::tempdir().unwrap();
    let workspace = workspace_dir.path().to_path_buf();
    let wrapped = MediaArtifactTool::new(
        StubMediaTool {
            file_count: 0,
            fail: false,
        },
        ArtifactKind::Video,
        workspace.clone(),
        workspace.join("Files"),
    );
    assert_eq!(wrapped.name(), "stub_generate");
    assert_eq!(wrapped.permission_level(), PermissionLevel::ReadOnly);
    assert_eq!(wrapped.category(), ToolCategory::System);
}

/// A video-like tool whose poll observer is the one its call was built with:
/// it polls a job state, then waits like a long job.
struct PollingTool {
    observer: tinyagents_harness::tinyinference_video::ProgressFn,
    state: tinyagents_harness::tinyinference_video::JobState,
}

#[async_trait]
impl Tool for PollingTool {
    fn name(&self) -> &str {
        "polling_generate"
    }
    fn description(&self) -> &str {
        "polling"
    }
    fn parameters_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, tinytools::ToolCallOptions::default(), None)
            .await
    }
    async fn execute_with_context(
        &self,
        _args: Value,
        _options: tinytools::ToolCallOptions,
        _context: Option<&dyn tinytools::ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        (self.observer)(&tinyagents_harness::tinyinference_video::VideoJobStatus {
            id: "job".into(),
            state: self.state.clone(),
            outputs: 0,
            output_indices: Vec::new(),
            cost_usd: None,
            error: None,
        });
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        Ok(ToolResult::error("no files"))
    }
}

#[derive(Default)]
struct Recorder(std::sync::Mutex<Vec<String>>);

impl tinytools::ToolRunContext for Recorder {
    fn report_progress(&self, update: tinytools::ToolProgress) {
        self.0
            .lock()
            .unwrap()
            .push(update.message.unwrap_or_default());
    }
}

/// The per-call inner tool binds the poll observer to the call it serves, so
/// two concurrent video calls each report their own job state.
#[tokio::test(start_paused = true)]
async fn per_call_observer_reports_each_calls_own_job_state() {
    use tinyagents_harness::tinyinference_video::JobState;
    let root = tempfile::tempdir().unwrap();
    let states = std::sync::Mutex::new(vec![JobState::InProgress, JobState::Pending]);
    let wrapped = MediaArtifactTool::new(
        PollingTool {
            observer: std::sync::Arc::new(|_| {}),
            state: JobState::Pending,
        },
        ArtifactKind::Video,
        root.path().join("workspace"),
        root.path().join("Files"),
    )
    .with_per_call_inner(move |observer, _chat| PollingTool {
        observer,
        state: states.lock().unwrap().pop().unwrap(),
    });
    let (a, b) = (Recorder::default(), Recorder::default());
    let (ra, rb) = tokio::join!(
        wrapped.execute_with_context(json!({}), tinytools::ToolCallOptions::default(), Some(&a)),
        wrapped.execute_with_context(json!({}), tinytools::ToolCallOptions::default(), Some(&b)),
    );
    ra.unwrap();
    rb.unwrap();
    let (ma, mb) = (a.0.lock().unwrap().clone(), b.0.lock().unwrap().clone());
    assert_eq!(ma, vec!["Generating video — 15s elapsed (job pending)"]);
    assert_eq!(mb, vec!["Generating video — 15s elapsed (job in_progress)"]);
}
