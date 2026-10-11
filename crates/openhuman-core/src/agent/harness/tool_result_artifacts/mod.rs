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
//! * **where the files go**: `<workspace_dir>/artifacts/tool-results`
//!   ([`crate::security::policy::tool_result_artifacts_dir`]), a detached store
//!   outside the action directory, so a tool output is never written into the
//!   project the agent is working in.

use serde_json::Value;
use tinyagents_harness::artifacts::tool_results::{self, ArtifactRead, ToolResultArtifactStore};

use crate::agent::harness::artifact_offload::{SanitizingRedactor, READ_TOOL};
use tinytools_std::filesystem::FileReadTool;

/// Namespace of the artifact index in the run's store registry.
pub(crate) const TINYAGENTS_TOOL_RESULT_ARTIFACT_STORE: &str = "openhuman_tool_result_artifacts";

/// A detached store for one session, keeping its files in `storage_dir` (see
/// [`crate::security::policy::tool_result_artifacts_dir`]) and wired to
/// OpenHuman's redactor, read tool and read limit.
pub(crate) fn new_tool_result_store(
    storage_dir: std::path::PathBuf,
    session_key: impl Into<String>,
) -> ToolResultArtifactStore {
    ToolResultArtifactStore::detached(
        storage_dir,
        session_key,
        std::sync::Arc::new(SanitizingRedactor),
        READ_TOOL,
        FileReadTool::MAX_FILE_SIZE_BYTES,
    )
}

/// The artifact a tool call reads, if any — `file_read`, possibly wrapped in
/// `use_skill` (#6284).
///
/// With a `store`, its own pointers are recognised (a detached store's are
/// absolute); without one, only the relative `artifacts/tool-results/…` form a
/// store-less turn can still meet in an older transcript.
pub(crate) fn artifact_read_target(
    store: Option<&ToolResultArtifactStore>,
    tool_name: &str,
    args: &Value,
) -> Option<ArtifactRead> {
    let wrapper = tinyagents_harness::tool::packs::USE_SKILL;
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
