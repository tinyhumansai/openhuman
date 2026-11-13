//! [`MediaArtifactTool`] — wraps a media-generation [`Tool`] (TinyAgents'
//! `GenerateImageTool` / `GenerateVideoTool`, boxed in `super::tools`) so
//! every file the inner tool saves is also tracked as an OpenHuman artifact,
//! following the pattern `tools/impl/document/mod.rs` and
//! `tools/impl/presentation/mod.rs` use for their own producers
//! (`create_artifact` → write bytes → `finalize_artifact`/`fail_artifact`).
//!
//! The inner tool already writes bytes under
//! `<action_dir>/generated-media/...` (see [`super::tools::media_tools_from`])
//! and reports each saved file in its JSON result's `artifacts` array
//! (`{"type": "image"|"video", "path", "media_type"|"format", "bytes"}`, see
//! `tinyagents_harness::media::{GenerateImageTool, GenerateVideoTool}`). This
//! wrapper runs the inner tool unchanged, then for every reported file:
//! reserves an artifact via `create_artifact_for_call` (recording the
//! provider-assigned tool-call id so `ArtifactPending`/`Ready`/`Failed`
//! correlate with the tool-call bubble, #C5), moves the file into the
//! artifact's reserved path, and finalizes it — or fails just that one
//! artifact and annotates its entry with `artifact_error`, without
//! disturbing the others (`n > 1` generations are common for images). On
//! success each entry gains an `artifact_id` field so the model (and the
//! bridged `chat.tool_result`) can reference the card; the rest of the
//! tool's JSON/markdown result shape is preserved byte-for-byte.
//!
//! A run with no context (`execute`/`execute_with_options`, e.g. CLI/tests)
//! still creates artifacts — `create_artifact_for_call` simply records no
//! `tool_call_id` and the chat-context bridge in `agent/artifacts/store.rs`
//! degrades to `thread_id = None` as it already does for every other
//! producer.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{json, Value};
use tinytools::{
    PermissionLevel, Tool, ToolCallOptions, ToolCategory, ToolContent, ToolPolicy, ToolResult,
    ToolRunContext, ToolScope, ToolTimeout,
};

use crate::agent::artifacts::{
    create_artifact_for_call, fail_artifact, finalize_artifact, ArtifactKind,
};
use crate::security::approval::ApprovalChatContext;
use crate::tools::host_extensions::tool_call_id;
use tinyagents_harness::tinyinference_video::ProgressFn;

use super::progress::{video_status_observer, JobStateSlot};

/// Maximum characters of the prompt kept in a generated artifact's title.
const TITLE_PROMPT_CHARS: usize = 60;

/// Wraps a media-generation tool so every file it saves is also tracked as
/// an OpenHuman artifact. See module docs for the flow.
pub struct MediaArtifactTool<T: Tool> {
    inner: T,
    kind: ArtifactKind,
    workspace_dir: PathBuf,
    /// Visible folder each generated file is moved into (#5505).
    files_dir: crate::agent::artifacts::FileRoots,
    /// Builds the inner tool for one call around that call's poll observer.
    /// Set for the video tool, whose `WaitPolicy::progress` hook is fixed at
    /// construction: a tool per call lets the observer record into the call's
    /// own [`JobStateSlot`]. `inner` still answers the static `Tool` metadata.
    per_call: Option<PerCallInner<T>>,
}

type PerCallInner<T> =
    std::sync::Arc<dyn Fn(ProgressFn, Option<&ApprovalChatContext>) -> T + Send + Sync>;

impl<T: Tool> MediaArtifactTool<T> {
    /// `kind` is the artifact category to file every generated output
    /// under (`ArtifactKind::Image` / `ArtifactKind::Video`); `workspace_dir`
    /// holds the artifact metadata; `files_dir` is the visible folder the
    /// generated file is moved into (#5505).
    pub fn new(
        inner: T,
        kind: ArtifactKind,
        workspace_dir: impl Into<PathBuf>,
        files_dir: impl Into<crate::agent::artifacts::FileRoots>,
    ) -> Self {
        Self {
            inner,
            kind,
            workspace_dir: workspace_dir.into(),
            files_dir: files_dir.into(),
            per_call: None,
        }
    }

    /// Run each call on a tool built by `build` around that call's poll
    /// observer and chat routing, so a long job's polled state reaches that
    /// call's heartbeat and its egress disclosures reach that call's chat.
    #[must_use]
    pub fn with_per_call_inner(
        mut self,
        build: impl Fn(ProgressFn, Option<&ApprovalChatContext>) -> T + Send + Sync + 'static,
    ) -> Self {
        self.per_call = Some(std::sync::Arc::new(build));
        self
    }

    /// Derives a human-readable artifact title from the call's `prompt`
    /// argument, suffixed with `(i/n)` when the call produced more than one
    /// file (`n > 1`).
    fn artifact_title(args: &Value, index: usize, total: usize) -> String {
        let prompt = args
            .get("prompt")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let base = prompt.map_or_else(
            || "Generated media".to_string(),
            |p| p.chars().take(TITLE_PROMPT_CHARS).collect(),
        );
        if total > 1 {
            format!("{base} ({}/{})", index + 1, total)
        } else {
            base
        }
    }

    /// Walks the inner tool's `artifacts` array (if any) and files each
    /// entry as an OpenHuman artifact in place, adding `artifact_id` on
    /// success or `artifact_error` on failure. Any content block shape the
    /// wrapper does not recognise (no JSON block, or JSON with no
    /// `artifacts` array) is returned unmodified.
    async fn attach_artifacts(
        &self,
        mut result: ToolResult,
        args: &Value,
        call_id: Option<String>,
        chat: Option<&ApprovalChatContext>,
    ) -> ToolResult {
        let Some(data) = result.content.iter_mut().find_map(|block| match block {
            ToolContent::Json { data } => Some(data),
            _ => None,
        }) else {
            return result;
        };
        let Some(artifacts) = data.get_mut("artifacts").and_then(Value::as_array_mut) else {
            return result;
        };
        let total = artifacts.len();
        for (index, entry) in artifacts.iter_mut().enumerate() {
            let Some(src_path) = entry.get("path").and_then(Value::as_str).map(PathBuf::from)
            else {
                continue;
            };
            self.file_one(
                entry,
                &src_path,
                args,
                index,
                total,
                call_id.as_deref(),
                chat,
            )
            .await;
        }
        result
    }

    async fn file_one(
        &self,
        entry: &mut Value,
        src: &Path,
        args: &Value,
        index: usize,
        total: usize,
        call_id: Option<&str>,
        chat: Option<&ApprovalChatContext>,
    ) {
        let ext = src
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("bin")
            .to_string();
        let title = Self::artifact_title(args, index, total);
        let (meta, dest) = match create_artifact_for_call(
            &self.workspace_dir,
            &self.files_dir,
            self.kind.clone(),
            &title,
            &ext,
            call_id,
            chat,
        )
        .await
        {
            Ok(pair) => pair,
            Err(err) => {
                tracing::warn!(
                    target: "media_generation",
                    err = %err,
                    "[media_generation] create_artifact failed; generated file left unfiled"
                );
                set_artifact_error(entry, &err);
                return;
            }
        };
        match move_or_copy(src, &dest).await {
            Ok(size_bytes) => {
                match finalize_artifact(&self.workspace_dir, &meta.id, size_bytes, chat).await {
                    Ok(updated) => {
                        if let Some(obj) = entry.as_object_mut() {
                            obj.insert("artifact_id".into(), json!(updated.id));
                        }
                    }
                    Err(err) => {
                        let _ = fail_artifact(
                            &self.workspace_dir,
                            &self.files_dir,
                            &meta.id,
                            &err,
                            chat,
                        )
                        .await;
                        set_artifact_error(entry, &err);
                    }
                }
            }
            Err(err) => {
                let _ =
                    fail_artifact(&self.workspace_dir, &self.files_dir, &meta.id, &err, chat).await;
                tracing::warn!(
                    target: "media_generation",
                    err = %err,
                    artifact_id = %meta.id,
                    "[media_generation] failed to file generated media as an artifact"
                );
                set_artifact_error(entry, &err);
            }
        }
    }
}

fn set_artifact_error(entry: &mut Value, message: &str) {
    if let Some(obj) = entry.as_object_mut() {
        obj.insert("artifact_error".into(), json!(message));
    }
}

/// Moves `src` to `dest`, falling back to copy + remove across filesystem
/// boundaries (`rename` fails with `EXDEV` when the files folder and the
/// media output dir are on different mounts). Returns the final file size.
async fn move_or_copy(src: &Path, dest: &Path) -> Result<u64, String> {
    if tokio::fs::rename(src, dest).await.is_err() {
        tokio::fs::copy(src, dest).await.map_err(|e| {
            format!(
                "failed to copy {} -> {}: {e}",
                src.display(),
                dest.display()
            )
        })?;
        let _ = tokio::fs::remove_file(src).await;
    }
    let stat = tokio::fs::metadata(dest)
        .await
        .map_err(|e| format!("failed to stat {}: {e}", dest.display()))?;
    Ok(stat.len())
}

#[async_trait]
impl<T: Tool> Tool for MediaArtifactTool<T> {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn parameters_schema(&self) -> Value {
        self.inner.parameters_schema()
    }

    fn policy(&self) -> ToolPolicy {
        self.inner.policy()
    }

    fn permission_level(&self) -> PermissionLevel {
        self.inner.permission_level()
    }

    fn permission_level_with_args(&self, args: &Value) -> PermissionLevel {
        self.inner.permission_level_with_args(args)
    }

    fn scope(&self) -> ToolScope {
        self.inner.scope()
    }

    fn category(&self) -> ToolCategory {
        self.inner.category()
    }

    fn external_effect(&self) -> bool {
        self.inner.external_effect()
    }

    fn external_effect_with_args(&self, args: &Value) -> bool {
        self.inner.external_effect_with_args(args)
    }

    fn timeout_policy(&self, args: &Value) -> ToolTimeout {
        self.inner.timeout_policy(args)
    }

    fn supports_markdown(&self) -> bool {
        self.inner.supports_markdown()
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_options(
        &self,
        args: Value,
        options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, options, None).await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        options: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        // Generation blocks for minutes; report a heartbeat (with the video
        // job's polled state) so the wait is visible on the call.
        let label = match self.kind {
            ArtifactKind::Video => "Generating video",
            _ => "Generating image",
        };
        let job_state = JobStateSlot::default();
        let approval = crate::agent::tinyagents::host::run_context::approval_for_tool(context);
        let per_call_inner = self.per_call.as_ref().map(|build| {
            build(
                video_status_observer(job_state.clone()),
                approval.chat.as_ref(),
            )
        });
        let inner = per_call_inner.as_ref().unwrap_or(&self.inner);
        let result = super::progress::with_progress_heartbeat(
            context,
            label,
            super::progress::HEARTBEAT_INTERVAL,
            job_state,
            inner.execute_with_context(args.clone(), options, context),
        )
        .await?;
        if result.is_error {
            return Ok(result);
        }
        let call_id = tool_call_id(context);
        Ok(self
            .attach_artifacts(result, &args, call_id, approval.chat.as_ref())
            .await)
    }
}

#[cfg(test)]
#[path = "artifact_tool_tests.rs"]
mod tests;
