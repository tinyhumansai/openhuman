//! [`MediaArtifactTool`] — wraps a media-generation [`Tool`] (TinyAgents'
//! `GenerateImageTool` / `GenerateVideoTool`, boxed in `super::tools`) so
//! every file the inner tool saves is also filed as a versioned OpenHuman
//! artifact, through the same [`Production`] flow the document and
//! presentation producers use.
//!
//! The inner tool writes bytes to its `MediaOutput` blob store and reports
//! each blob key in its JSON result's `artifacts` array
//! (`{"type": "image"|"video", "path", "media_type"|"format", "bytes"}`, see
//! `tinyagents_harness::media::{GenerateImageTool, GenerateVideoTool}`). Host
//! adapters may also report absolute file paths. This wrapper runs the inner
//! tool unchanged, then for every reported output: starts a [`Production`]
//! (recording the provider-assigned tool-call id so the card correlates with
//! the tool-call bubble, #C5), stores the blob or file into the artifact's
//! reserved path, and completes it — or fails just that one
//! artifact and annotates its entry with `artifact_error`, without
//! disturbing the others (`n > 1` generations are common for images). On
//! success each entry gains an `artifact_id` field so the model (and the
//! bridged `chat.tool_result`) can reference the card; the rest of the
//! tool's JSON/markdown result shape is preserved byte-for-byte.
//!
//! A run with no context (`execute`/`execute_with_options`, e.g. CLI/tests)
//! still creates artifacts — the production simply records no
//! `tool_call_id` and the chat-context bridge in `agent/artifacts/store.rs`
//! degrades to `thread_id = None` as it already does for every other
//! producer.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinytools::{
    PermissionLevel, Tool, ToolCallOptions, ToolCategory, ToolContent, ToolPolicy, ToolResult,
    ToolRunContext, ToolScope, ToolTimeout,
};
use tokio::io::AsyncWriteExt;

use crate::agent::artifacts::production::Production;
use crate::agent::artifacts::ArtifactKind;
use crate::security::approval::ApprovalChatContext;
use crate::tools::host_extensions::tool_call_id;
use tinyagents_harness::tinyinference_video::ProgressFn;

use super::progress::{video_status_observer, JobStateSlot};

const FILE_REFERENCE_FIELDS: [&str; 8] = [
    "references",
    "reference_images",
    "input_images",
    "inputImages",
    "first_frame",
    "inputImage",
    "input_image",
    "last_frame",
];

/// Whether a media call references an absolute path under a trusted Files root.
/// Only these requests need a durable-artifact refresh before tool execution.
fn has_reusable_files_reference(args: &Value, files: &crate::agent::artifacts::FileRoots) -> bool {
    for field in FILE_REFERENCE_FIELDS {
        let Some(value) = args.get(field) else {
            continue;
        };
        let candidates = value.as_str().into_iter().chain(
            value
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str),
        );
        if candidates.into_iter().any(|candidate| {
            let path = Path::new(candidate);
            path.is_absolute() && files.contains_path(path)
        }) {
            return true;
        }
    }
    false
}

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
    blobs: Option<Arc<dyn tinystoragedrivers::BlobStore>>,
    allowed_references: Option<super::tools::MediaReferenceAllowlist>,
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
            blobs: None,
            allowed_references: None,
            per_call: None,
        }
    }

    /// Reads generated outputs addressed by blob key from the media tool's
    /// backing store. This keeps TinyAgents' blob-backed output contract
    /// separate from the host's visible Files-folder artifact store.
    #[must_use]
    pub fn with_blobs(mut self, blobs: Arc<dyn tinystoragedrivers::BlobStore>) -> Self {
        self.blobs = Some(blobs);
        self
    }

    /// Allows later media calls to use only the exact files this wrapper filed.
    #[must_use]
    pub(crate) fn with_reference_allowlist(
        mut self,
        allowed_references: super::tools::MediaReferenceAllowlist,
    ) -> Self {
        self.allowed_references = Some(allowed_references);
        self
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
        let mut filed_paths = Vec::new();
        for (index, entry) in artifacts.iter_mut().enumerate() {
            let Some(src_path) = entry.get("path").and_then(Value::as_str).map(PathBuf::from)
            else {
                continue;
            };
            if let Some(filed_path) = self
                .file_one(
                    entry,
                    &src_path,
                    args,
                    index,
                    total,
                    call_id.as_deref(),
                    chat,
                )
                .await
            {
                filed_paths.push((src_path.to_string_lossy().into_owned(), filed_path));
            }
        }
        if let Some(markdown) = result.markdown_formatted.as_mut() {
            for (source, destination) in filed_paths {
                *markdown = markdown.replace(&source, &destination);
            }
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
    ) -> Option<String> {
        let ext = src
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("bin")
            .to_string();
        let title = Self::artifact_title(args, index, total);
        let kind = match self.kind {
            ArtifactKind::Image => tiny_artifacts::ArtifactKind::Image,
            _ => tiny_artifacts::ArtifactKind::File,
        };
        // Reserve the file in the files folder (the card shows
        // "generating…", tied to this tool call's bubble) and open the
        // versioned artifact under the same id.
        let production = match Production::start(
            &self.workspace_dir,
            self.files_dir.clone(),
            kind,
            &title,
            &ext,
            call_id,
            None,
            chat,
        )
        .await
        {
            Ok(production) => production,
            Err(err) => {
                tracing::warn!(
                    target: "media_generation",
                    err = %err,
                    "[media_generation] could not open the artifact; generated file left unfiled"
                );
                set_artifact_error(entry, &err);
                return None;
            }
        };
        let dest = production.path().to_path_buf();
        let stored = if src.is_absolute() {
            move_or_copy(src, &dest).await
        } else {
            self.file_blob(src, &dest).await
        };
        if let Err(err) = stored {
            production.fail(&err).await;
            tracing::warn!(
                target: "media_generation",
                err = %err,
                artifact_id = %production.id(),
                "[media_generation] failed to file generated media as an artifact"
            );
            set_artifact_error(entry, &err);
            return None;
        }
        let canonical = dest.canonicalize().ok();
        let filed_path = canonical.as_deref().unwrap_or(&dest).display().to_string();
        if let Some(obj) = entry.as_object_mut() {
            obj.insert("path".into(), json!(filed_path));
        }
        // The stored file becomes the artifact's first version; the card turns
        // ready from that record.
        match self.complete_and_discard_blob(&production, src).await {
            Ok(_) => {
                if let (Some(allowed), Some(canonical)) = (&self.allowed_references, canonical) {
                    allowed
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .insert(canonical);
                }
                if let Some(obj) = entry.as_object_mut() {
                    obj.insert("artifact_id".into(), json!(production.id()));
                }
                Some(filed_path)
            }
            Err(err) => {
                set_artifact_error(entry, &err);
                None
            }
        }
    }

    async fn delete_blob_after_finalize(&self, src: &Path) {
        if src.is_absolute() {
            return;
        }
        let (Some(blobs), Some(key)) = (self.blobs.as_ref(), src.to_str()) else {
            return;
        };
        if let Err(error) = blobs.delete(key).await {
            tracing::warn!(
                target: "media_generation",
                %error,
                blob_key = key,
                "[media_generation] finalized media but could not discard its source blob"
            );
        }
    }

    /// Complete `production` and only then discard its source blob, so a
    /// failed completion leaves the blob for a retry.
    async fn complete_and_discard_blob(
        &self,
        production: &Production,
        src: &Path,
    ) -> Result<u64, String> {
        let size = production.complete().await?;
        self.delete_blob_after_finalize(src).await;
        Ok(size)
    }

    /// Rehydrates finalized media paths after a session rebuild. Only Ready
    /// image/video artifacts whose files still resolve through the configured
    /// trusted roots enter the exact-path allowlist.
    async fn refresh_allowed_references(&self) -> Result<(), String> {
        let Some(allowed) = &self.allowed_references else {
            return Ok(());
        };

        let mut recovered = std::collections::HashSet::new();
        let previously_allowed = allowed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let artifacts = match crate::agent::artifacts::store::list_artifacts(
            &self.workspace_dir,
            0,
            usize::MAX,
            None,
        )
        .await
        {
            Ok((artifacts, _)) => artifacts,
            Err(error) => {
                tracing::warn!(
                    target: "media_generation",
                    %error,
                    "[media_generation] could not refresh reusable artifact paths"
                );
                return Err(
                    "could not validate previously generated media references; retry the media call"
                        .to_string(),
                );
            }
        };

        for meta in artifacts.iter().filter(|meta| {
            matches!(&meta.kind, ArtifactKind::Image | ArtifactKind::Video)
                && matches!(&meta.status, crate::agent::artifacts::ArtifactStatus::Ready)
        }) {
            let Ok(path) = crate::agent::artifacts::resolve_ready_file_meta(
                &self.workspace_dir,
                &self.files_dir,
                meta,
            )
            .await
            else {
                continue;
            };
            if let Ok(path) = tokio::fs::canonicalize(path).await {
                recovered.insert(path);
            }
        }

        // Prune paths that were allowed before this scan but are no longer
        // Ready. Paths added concurrently after the snapshot are preserved.
        let mut allowed = allowed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        allowed.retain(|path| !previously_allowed.contains(path) || recovered.contains(path));
        allowed.extend(recovered);
        Ok(())
    }

    async fn file_blob(&self, key: &Path, dest: &Path) -> Result<u64, String> {
        let key = key
            .to_str()
            .ok_or_else(|| "generated media blob key is not valid UTF-8".to_string())?;
        let blobs = self.blobs.as_ref().ok_or_else(|| {
            format!("generated media returned blob key {key:?} without a blob store")
        })?;
        let mut blob = blobs
            .get_stream(key)
            .await
            .map_err(|error| format!("failed to read generated media blob {key:?}: {error}"))?
            .ok_or_else(|| format!("generated media blob {key:?} was not found"))?;
        copy_blob_to_file(&mut *blob.reader, dest, key).await
    }
}

async fn copy_blob_to_file<R>(reader: &mut R, dest: &Path, key: &str) -> Result<u64, String>
where
    R: tokio::io::AsyncRead + Unpin + ?Sized,
{
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(dest)
        .await
        .map_err(|error| {
            format!(
                "failed to open artifact destination {}: {error}",
                dest.display()
            )
        })?;
    let size = match tokio::io::copy(reader, &mut file).await {
        Ok(size) => size,
        Err(error) => {
            drop(file);
            let _ = tokio::fs::remove_file(dest).await;
            return Err(format!(
                "failed to copy generated media blob {key:?}: {error}"
            ));
        }
    };
    if let Err(error) = file.flush().await {
        drop(file);
        let _ = tokio::fs::remove_file(dest).await;
        return Err(format!(
            "failed to flush artifact destination {}: {error}",
            dest.display()
        ));
    }
    Ok(size)
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
        if has_reusable_files_reference(&args, &self.files_dir) {
            self.refresh_allowed_references()
                .await
                .map_err(anyhow::Error::msg)?;
        }

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
