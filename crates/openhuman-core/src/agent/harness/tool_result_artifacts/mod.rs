//! OpenHuman's wiring of per-tool-result artifact persistence.
//!
//! The mechanics — the store, the `[tool_result_preview]` envelope, the
//! per-result and aggregate budgets, paged artifact reads — live in
//! [`tinyagents_harness::artifacts::tool_results`]. What stays here is what only
//! the host can decide:
//!
//! * **the redactor**: the same `sanitize_text` pass that scrubs a worker
//!   artifact ([`SanitizingRedactor`]), because an artifact on disk is exactly as
//!   readable as the tool result it replaces;
//! * **the tool vocabulary**: `file_read` opens an artifact and `use_skill` is the
//!   one wrapper whose result *is* the wrapped tool's result;
//! * **the read limit**: the largest body `file_read` will open
//!   ([`FileReadTool::MAX_FILE_SIZE_BYTES`]);
//! * **where the bodies live**: on the acting tenant's scoped blob store
//!   ([`crate::storage::sessions::artifact_blobs_for`]), under the
//!   `tool-results/<session>/<tool>/<call>.txt` key the pointer quotes. Nothing
//!   is written into the project the agent is working in;
//! * **how `file_read` opens one**: `file_read` reads the filesystem, so a read
//!   of a `tool-results/…` key is first *materialised* from the blob store into
//!   `<workspace_dir>/artifacts/tool-results`
//!   ([`crate::security::policy::tool_result_artifacts_dir`], a read-only trusted
//!   root) and the call is pointed at that file ([`HostArtifactStore::resolve_read`]).
//!   The blob lookup runs on every read, so a key another tenant wrote is absent
//!   from this tenant's store and is never served, and a session that reloads
//!   finds its artifacts again because the blobs outlive the process.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use tinyagents_harness::artifacts::tool_results::{self, ArtifactRead, ToolResultArtifactStore};
use tinystoragedrivers::BlobStore;

use crate::agent::harness::artifact_offload::{SanitizingRedactor, READ_TOOL};
use tinytools_std::filesystem::FileReadTool;

/// Namespace of the artifact index in the run's store registry.
pub(crate) const TINYAGENTS_TOOL_RESULT_ARTIFACT_STORE: &str = "openhuman_tool_result_artifacts";

/// A session's artifact store plus the directory its bodies are materialised
/// into for `file_read`. Derefs to the harness [`ToolResultArtifactStore`].
#[derive(Clone, Debug)]
pub(crate) struct HostArtifactStore {
    store: ToolResultArtifactStore,
    materialize_root: PathBuf,
}

impl std::ops::Deref for HostArtifactStore {
    type Target = ToolResultArtifactStore;
    fn deref(&self) -> &Self::Target {
        &self.store
    }
}

/// A store for one session over `blobs`, wired to OpenHuman's redactor, read
/// tool and read limit. `materialize_root` is where `file_read` opens bodies
/// (see [`crate::security::policy::tool_result_artifacts_dir`]).
pub(crate) fn new_tool_result_store(
    blobs: Arc<dyn BlobStore>,
    materialize_root: PathBuf,
    session_key: impl Into<String>,
) -> HostArtifactStore {
    HostArtifactStore {
        store: ToolResultArtifactStore::new(
            blobs,
            session_key,
            Arc::new(SanitizingRedactor),
            READ_TOOL,
            FileReadTool::MAX_FILE_SIZE_BYTES,
        ),
        materialize_root,
    }
}

impl HostArtifactStore {
    /// Apply a retention to every artifact written from here on.
    pub(crate) fn with_retention(mut self, retention: std::time::Duration) -> Self {
        self.store = self.store.with_retention(retention);
        self
    }

    /// The directory bodies are materialised into.
    pub(crate) fn materialize_root(&self) -> &Path {
        &self.materialize_root
    }

    /// Write the stored body of `key` under the materialise root and return its
    /// path, or `None` when this tenant's store has no such artifact. The key
    /// was validated fail-closed by [`ToolResultArtifactStore::read`], so the
    /// join cannot leave the root.
    pub(crate) async fn materialize(&self, key: &str) -> anyhow::Result<Option<PathBuf>> {
        let Some(body) = self.store.read(key).await? else {
            return Ok(None);
        };
        let relative = relative_to_namespace(key);
        let target = self.materialize_root.join(relative);
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&target, body).await?;
        Ok(Some(target))
    }

    /// Point a `file_read` of an artifact key at its materialised file, in
    /// place. Leaves the arguments untouched when the artifact is absent
    /// (never written, expired, purged or another tenant's).
    pub(crate) async fn resolve_read(&self, args: &mut Value, read: &ArtifactRead) {
        match self.materialize(&read.path).await {
            Ok(Some(target)) => {
                tracing::debug!(
                    key = %read.path,
                    "[tool-result-artifacts] materialised blob for file_read"
                );
                set_read_path(args, &target.to_string_lossy());
            }
            Ok(None) => tracing::debug!(
                key = %read.path,
                "[tool-result-artifacts] no such artifact in this tenant's store"
            ),
            Err(error) => tracing::warn!(
                key = %read.path,
                error = %error,
                "[tool-result-artifacts] could not materialise artifact (continuing)"
            ),
        }
    }
}

/// The part of a key below the tool-results namespace: the harness's
/// `harness/tool_results/…` (v2), `tool-results/…` (v1) or the relative
/// `artifacts/tool-results/…` form.
fn relative_to_namespace(key: &str) -> &str {
    let key = key.trim_start_matches("./");
    key.strip_prefix("artifacts/tool-results/")
        .or_else(|| key.strip_prefix("harness/tool_results/"))
        .or_else(|| key.strip_prefix("tool-results/"))
        .unwrap_or(key)
}

/// Set the `path` of the read call in `args`, following `use_skill`'s
/// `{"tool", "args"}` wrapper to the call it runs.
fn set_read_path(args: &mut Value, path: &str) {
    let mut cursor = args;
    while cursor.get("tool").is_some() && cursor.get("args").is_some_and(Value::is_object) {
        cursor = &mut cursor["args"];
    }
    if let Some(object) = cursor.as_object_mut() {
        object.insert("path".to_string(), Value::String(path.to_string()));
    }
}

/// A store over a private in-memory blob store, materialising under
/// `materialize_root`: the tests' stand-in for a tenant's scoped storage.
#[cfg(test)]
pub(crate) fn in_memory_tool_result_store(
    materialize_root: PathBuf,
    session_key: impl Into<String>,
) -> HostArtifactStore {
    let scope = tinystoragedrivers::Scope::new("local").expect("scope");
    let blobs = tinystoragedrivers::MemoryStorage::new();
    let blobs = tinystoragedrivers::StorageBackend::for_scope(&blobs, &scope).expect("scope");
    new_tool_result_store(Arc::clone(blobs.blobs()), materialize_root, session_key)
}

/// The artifact a tool call reads, if any — `file_read`, possibly wrapped in
/// `use_skill` (#6284).
///
/// With a `store`, its own `tool-results/…` keys are recognised; without one,
/// only the legacy relative `artifacts/tool-results/…` form a store-less turn
/// can still meet in an older transcript.
pub(crate) fn artifact_read_target(
    store: Option<&ToolResultArtifactStore>,
    tool_name: &str,
    args: &Value,
) -> Option<ArtifactRead> {
    let wrapper = tinyagents_harness_tools::skills::USE_SKILL;
    match store {
        Some(store) => store.read_target(tool_name, args, wrapper),
        None => tool_results::artifact_read_target(tool_name, args, READ_TOOL, wrapper),
    }
}

/// Bound one page of an artifact read to `budget_bytes`.
pub(crate) fn page_artifact_read(
    content: String,
    read: &ArtifactRead,
    budget_bytes: usize,
) -> String {
    tool_results::page_artifact_read(content, read, budget_bytes, READ_TOOL)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
