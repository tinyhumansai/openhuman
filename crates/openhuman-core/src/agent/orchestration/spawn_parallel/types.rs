//! Shared worker and result types passed between the staging, worker-fanout,
//! and collection phases.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tinytools::WorkspaceDescriptor;

use crate::agent::harness::host_definition::AgentDefinition;

/// One worker admitted by the `spawn_parallel_agents` tool.
///
/// This is intentionally a host request contract: its ownership syntax and
/// worktree options are OpenHuman product policy. The tool owns JSON
/// decoding; the execution pipeline receives this typed value only. A legacy
/// `toolkit` key from an older caller is ignored (unknown fields are not
/// denied).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ParallelAgentTask {
    pub(crate) agent_id: String,
    pub(crate) prompt: String,
    #[serde(default)]
    pub(crate) context: Option<String>,
    #[serde(default)]
    pub(crate) ownership: Option<String>,
    #[serde(default)]
    pub(crate) isolation: Option<String>,
    #[serde(default)]
    pub(crate) base_ref: Option<String>,
}

/// Host payload of a module `Worker`: resolved definition, lineage and worktree
/// placement. Id, prompt (ownership boundary applied) and dispatch mode live on
/// the module's `Worker`.
#[derive(Clone)]
pub(crate) struct SpawnParallelWorker {
    pub(crate) definition: AgentDefinition,
    pub(crate) task: ParallelAgentTask,
    pub(crate) lineage: ParallelAgentLineage,
    pub(crate) worktree_path: Option<PathBuf>,
    pub(crate) workspace_descriptor: Option<WorkspaceDescriptor>,
}

/// Terminal or suspended lifecycle state projected by one parallel worker.
/// `success` remains the concise aggregate flag, while this preserves enough
/// detail to avoid treating a pause or cancellation as a completed child.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ParallelAgentStatus {
    Completed,
    AwaitingUser,
    Incomplete,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ParallelAgentLineage {
    pub(crate) parent_session: String,
    pub(crate) root_session: String,
    pub(crate) child_task_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ParallelAgentResult {
    pub(crate) task_id: String,
    pub(crate) agent_id: String,
    pub(crate) lineage: ParallelAgentLineage,
    pub(crate) success: bool,
    pub(crate) status: ParallelAgentStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) awaiting_question: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) checkpoint_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) ownership: Option<String>,
    pub(crate) elapsed_ms: u64,
    pub(crate) iterations: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) stale_parent_reads: Vec<String>,
    /// Absolute path to the worker's isolated `git worktree` checkout, when
    /// it ran with `isolation = "worktree"`. `None` for non-isolated workers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) worktree_path: Option<String>,
    /// Files (relative to the worktree root) the worker changed, collected
    /// from `git status` after the run. Empty for non-isolated workers or a
    /// clean worktree.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) changed_files: Vec<String>,
    /// Whether the worker's worktree had uncommitted changes after the run.
    /// A dirty worktree must not be auto-removed (surfaced to the UI so the
    /// user can choose). `None` for non-isolated workers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) dirty_status: Option<bool>,
    /// True only when this worker invocation committed the neutral lifecycle
    /// record, and therefore owns terminal event/progress publication.
    #[serde(skip)]
    pub(crate) emit_lifecycle_effects: bool,
}
