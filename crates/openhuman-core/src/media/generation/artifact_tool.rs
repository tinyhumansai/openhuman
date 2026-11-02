//! OpenHuman's media-generation adapters around TinyAgents' generic artifact tool.
//!
//! TinyAgents owns the result traversal, per-call tool construction, and progress
//! heartbeat. OpenHuman supplies the artifact store, file placement, blob access,
//! and the policy for reusing previously generated files as references.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tinyagents_harness_tools::media::{
    ArtifactFiling, FiledArtifact, GeneratedArtifact, MediaArtifactKind, MediaInnerFactory,
    MediaProgressFn,
};
use tinytools::{
    PermissionLevel, Tool, ToolCallOptions, ToolCategory, ToolPolicy, ToolResult, ToolRunContext,
    ToolScope, ToolTimeout,
};
use tokio::io::AsyncWriteExt;

use crate::artifacts::production::Production;
use crate::artifacts::ArtifactKind;
use tinystoragedrivers_api::BlobStore;

/// A host adapter that retains OpenHuman artifact behavior while delegating
/// generic media result handling to TinyAgents' harness-tools crate.
pub struct MediaArtifactTool<T: Tool> {
    inner: tinyagents_harness_tools::media::MediaArtifactTool<ReferenceRefreshTool<T>>,
    policy: Arc<RwLock<MediaArtifactPolicy>>,
    workspace_dir: PathBuf,
    files_dir: crate::artifacts::FileRoots,
    allowed_references: Option<super::tools::MediaReferenceAllowlist>,
}

#[derive(Clone)]
struct MediaArtifactPolicy {
    kind: ArtifactKind,
    workspace_dir: PathBuf,
    files_dir: crate::artifacts::FileRoots,
    blobs: Option<Arc<dyn BlobStore>>,
    allowed_references: Option<super::tools::MediaReferenceAllowlist>,
}

impl<T: Tool + 'static> MediaArtifactTool<T> {
    /// Creates the host adapter for a media generation tool.
    pub fn new(
        inner: T,
        kind: ArtifactKind,
        workspace_dir: impl Into<PathBuf>,
        files_dir: impl Into<crate::artifacts::FileRoots>,
    ) -> Self {
        let workspace_dir = workspace_dir.into();
        let files_dir = files_dir.into();
        let upstream_kind = match &kind {
            ArtifactKind::Image => MediaArtifactKind::Image,
            _ => MediaArtifactKind::Video,
        };
        let policy = Arc::new(RwLock::new(MediaArtifactPolicy {
            kind,
            workspace_dir: workspace_dir.clone(),
            files_dir: files_dir.clone(),
            blobs: None,
            allowed_references: None,
        }));
        let filing = Arc::new(OpenHumanArtifactFiling(Arc::clone(&policy)));
        let refreshing = ReferenceRefreshTool::new(inner, Arc::clone(&policy));
        let inner = tinyagents_harness_tools::media::MediaArtifactTool::new(
            refreshing,
            upstream_kind,
            filing,
        )
        .with_tool_call_id(Arc::new(crate::tools::host_extensions::tool_call_id));
        Self {
            inner,
            policy,
            workspace_dir,
            files_dir,
            allowed_references: None,
        }
    }

    /// Reads generated outputs addressed by blob key from the media tool's
    /// backing store.
    #[must_use]
    pub fn with_blobs(self, blobs: Arc<dyn BlobStore>) -> Self {
        self.policy
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .blobs = Some(blobs);
        self
    }

    /// Allows later media calls to use only the exact files this wrapper filed.
    #[must_use]
    pub(crate) fn with_reference_allowlist(
        mut self,
        allowed_references: super::tools::MediaReferenceAllowlist,
    ) -> Self {
        self.policy
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .allowed_references = Some(Arc::clone(&allowed_references));
        self.allowed_references = Some(allowed_references);
        self
    }

    async fn refresh_allowed_references(&self) -> Result<(), String> {
        let policy = self
            .policy
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        refresh_allowed_references(&policy).await
    }

    /// Builds the inner generator for one call around its provider progress
    /// observer and the call's own host context.
    #[must_use]
    pub fn with_per_call_inner(
        self,
        build: impl for<'a> Fn(MediaProgressFn, Option<&'a dyn ToolRunContext>) -> T
            + Send
            + Sync
            + 'static,
    ) -> Self {
        let Self {
            inner,
            policy,
            workspace_dir,
            files_dir,
            allowed_references,
        } = self;
        let build: MediaInnerFactory<T> = Arc::new(build);
        let per_call_policy = Arc::clone(&policy);
        let inner = inner.with_per_call_inner(Arc::new(move |progress, context| {
            ReferenceRefreshTool::new(build(progress, context), Arc::clone(&per_call_policy))
        }));
        Self {
            inner,
            policy,
            workspace_dir,
            files_dir,
            allowed_references,
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
impl<T: Tool + 'static> Tool for MediaArtifactTool<T> {
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

    fn max_result_size_chars(&self) -> Option<usize> {
        self.inner.max_result_size_chars()
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
        if let Err(error) = check_media_reference_floor(&args).await {
            return Ok(ToolResult::error(error));
        }
        self.inner
            .execute_with_context(args, options, context)
            .await
    }
}

async fn check_media_reference_floor(args: &Value) -> Result<(), String> {
    let mut paths = args
        .get("references")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    for key in ["first_frame", "last_frame"] {
        if let Some(path) = args.get(key).and_then(Value::as_str) {
            if !path.starts_with("https://") && !path.starts_with("http://") {
                paths.push(PathBuf::from(path));
            }
        }
    }
    if paths.is_empty() {
        return Ok(());
    }
    #[cfg(not(feature = "security-module"))]
    {
        Err(
            "[policy-blocked] TinySecurity path authorization unavailable; media reference refused"
                .into(),
        )
    }
    #[cfg(feature = "security-module")]
    {
        let protected = crate::modules::security::gap::protected_paths(&paths)
        .await
        .map_err(|error| {
            format!(
                "[policy-blocked] TinySecurity path authorization unavailable; media reference refused: {error}"
            )
        })?;
        if protected.len() != paths.len() || protected.iter().any(|denied| *denied) {
            return Err("[policy-blocked] media reference is in a protected location".into());
        }
        Ok(())
    }
}

/// OpenHuman's implementation of the TinyAgents artifact-filing port.
struct OpenHumanArtifactFiling(Arc<RwLock<MediaArtifactPolicy>>);

#[async_trait]
impl ArtifactFiling for OpenHumanArtifactFiling {
    async fn file(
        &self,
        generated: GeneratedArtifact,
        context: Option<&dyn ToolRunContext>,
    ) -> Result<FiledArtifact, String> {
        let policy = self
            .0
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let source = &generated.path;
        let extension = source
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("bin")
            .to_owned();
        let kind = match policy.kind {
            ArtifactKind::Image => tiny_artifacts::ArtifactKind::Image,
            _ => tiny_artifacts::ArtifactKind::File,
        };
        let approval = crate::agent::tinyagents::host::run_context::approval_for_tool(context);
        let production = Production::start(
            &policy.workspace_dir,
            policy.files_dir.clone(),
            kind,
            &generated.title,
            &extension,
            generated.tool_call_id.as_deref(),
            None,
            approval.chat.as_ref(),
        )
        .await
        .map_err(|error| {
            tracing::warn!(
                target = "media_generation",
                %error,
                "[media_generation] could not open the artifact; generated file left unfiled"
            );
            error
        })?;

        let destination = production.path().to_path_buf();
        let stored = if source.is_absolute() {
            move_or_copy(source, &destination).await
        } else {
            self.file_blob(&policy, source, &destination).await
        };
        if let Err(error) = stored {
            production.fail(&error).await;
            tracing::warn!(
                target = "media_generation",
                %error,
                artifact_id = %production.id(),
                "[media_generation] failed to file generated media as an artifact"
            );
            return Err(error);
        }

        let canonical = destination.canonicalize().ok();
        let filed_path = canonical
            .as_deref()
            .unwrap_or(&destination)
            .display()
            .to_string();
        match self.complete_and_discard_blob(&production, source).await {
            Ok(_) => {
                if let (Some(allowed), Some(canonical)) = (&policy.allowed_references, canonical) {
                    allowed
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .insert(canonical);
                }
                Ok(FiledArtifact {
                    id: production.id().to_string(),
                    path: filed_path,
                })
            }
            Err(error) => Err(error),
        }
    }
}

impl OpenHumanArtifactFiling {
    async fn delete_blob_after_finalize(&self, policy: &MediaArtifactPolicy, source: &Path) {
        if source.is_absolute() {
            return;
        }
        let (Some(blobs), Some(key)) = (policy.blobs.as_ref(), source.to_str()) else {
            return;
        };
        if let Err(error) = blobs.delete(key).await {
            tracing::warn!(
                target = "media_generation",
                %error,
                blob_key = key,
                "[media_generation] finalized media but could not discard its source blob"
            );
        }
    }

    /// Completes the artifact before discarding its source blob, so a failed
    /// completion leaves the blob available for retry.
    async fn complete_and_discard_blob(
        &self,
        production: &Production,
        source: &Path,
    ) -> Result<u64, String> {
        let size = production.complete().await?;
        let policy = self
            .0
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        self.delete_blob_after_finalize(&policy, source).await;
        Ok(size)
    }

    async fn file_blob(
        &self,
        policy: &MediaArtifactPolicy,
        key: &Path,
        destination: &Path,
    ) -> Result<u64, String> {
        let key = key
            .to_str()
            .ok_or_else(|| "generated media blob key is not valid UTF-8".to_owned())?;
        let blobs = policy.blobs.as_ref().ok_or_else(|| {
            format!("generated media returned blob key {key:?} without a blob store")
        })?;
        let mut blob = blobs
            .get_stream(key)
            .await
            .map_err(|error| format!("failed to read generated media blob {key:?}: {error}"))?
            .ok_or_else(|| format!("generated media blob {key:?} was not found"))?;
        copy_blob_to_file(&mut blob.reader, destination, key).await
    }
}

async fn copy_blob_to_file(
    reader: &mut tinyagents_storage_kit::blob::BlobReader,
    destination: &Path,
    key: &str,
) -> Result<u64, String> {
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(destination)
        .await
        .map_err(|error| {
            format!(
                "failed to open artifact destination {}: {error}",
                destination.display()
            )
        })?;
    let mut size = 0_u64;
    while let Some(chunk) = reader.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                drop(file);
                let _ = tokio::fs::remove_file(destination).await;
                return Err(format!(
                    "failed to copy generated media blob {key:?}: {error}"
                ));
            }
        };
        if let Err(error) = file.write_all(&chunk).await {
            drop(file);
            let _ = tokio::fs::remove_file(destination).await;
            return Err(format!(
                "failed to write generated media blob {key:?}: {error}"
            ));
        }
        size = size.saturating_add(chunk.len() as u64);
    }
    if let Err(error) = file.flush().await {
        drop(file);
        let _ = tokio::fs::remove_file(destination).await;
        return Err(format!(
            "failed to flush artifact destination {}: {error}",
            destination.display()
        ));
    }
    Ok(size)
}

/// Host policy wrapper that refreshes the allowlist before a generated-media
/// call references an absolute path under the Files root.
struct ReferenceRefreshTool<T: Tool> {
    inner: T,
    policy: Arc<RwLock<MediaArtifactPolicy>>,
}

impl<T: Tool> ReferenceRefreshTool<T> {
    fn new(inner: T, policy: Arc<RwLock<MediaArtifactPolicy>>) -> Self {
        Self { inner, policy }
    }

    async fn refresh_allowed_references(&self) -> Result<(), String> {
        let policy = self
            .policy
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        refresh_allowed_references(&policy).await
    }
}

async fn refresh_allowed_references(policy: &MediaArtifactPolicy) -> Result<(), String> {
    let Some(allowed) = &policy.allowed_references else {
        return Ok(());
    };

    let previously_allowed = allowed
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let artifacts =
        match crate::artifacts::store::list_artifacts(&policy.workspace_dir, 0, usize::MAX, None)
            .await
        {
            Ok((artifacts, _)) => artifacts,
            Err(error) => {
                tracing::warn!(
                    target = "media_generation",
                    %error,
                    "[media_generation] could not refresh reusable artifact paths"
                );
                return Err(
                "could not validate previously generated media references; retry the media call"
                    .to_owned(),
            );
            }
        };

    let mut recovered = std::collections::HashSet::new();
    for artifact in artifacts.iter().filter(|artifact| {
        matches!(&artifact.kind, ArtifactKind::Image | ArtifactKind::Video)
            && matches!(&artifact.status, crate::artifacts::ArtifactStatus::Ready)
    }) {
        let Ok(path) = crate::artifacts::resolve_ready_file_meta(
            &policy.workspace_dir,
            &policy.files_dir,
            artifact,
        )
        .await
        else {
            continue;
        };
        if let Ok(path) = tokio::fs::canonicalize(path).await {
            recovered.insert(path);
        }
    }

    let mut allowed = allowed
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    allowed.retain(|path| !previously_allowed.contains(path) || recovered.contains(path));
    allowed.extend(recovered);
    Ok(())
}

#[async_trait]
impl<T: Tool> Tool for ReferenceRefreshTool<T> {
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
    fn max_result_size_chars(&self) -> Option<usize> {
        self.inner.max_result_size_chars()
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
        if has_reusable_files_reference(
            &args,
            &self
                .policy
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .files_dir,
        ) {
            self.refresh_allowed_references()
                .await
                .map_err(anyhow::Error::msg)?;
        }
        self.inner
            .execute_with_context(args, options, context)
            .await
    }
}

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
fn has_reusable_files_reference(args: &Value, files: &crate::artifacts::FileRoots) -> bool {
    FILE_REFERENCE_FIELDS.iter().any(|field| {
        let Some(value) = args.get(field) else {
            return false;
        };
        let candidates = value.as_str().into_iter().chain(
            value
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str),
        );
        candidates.into_iter().any(|candidate| {
            let path = Path::new(candidate);
            path.is_absolute() && files.contains_path(path)
        })
    })
}

#[cfg(test)]
#[path = "artifact_tool_tests.rs"]
mod tests;
