//! Command-center view types for the background agent surface (issue #3373).
//!
//! The durable run ledger (`session_db::run_ledger`) stores fine-grained
//! `AgentRunStatus` values for every background agent run. The background
//! agent command center groups that work into five user-facing buckets so a
//! reviewer can see, at a glance, what needs input, what is still working, and
//! what finished, failed, or was stopped. These types are the read-only
//! projection the command center renders; nothing here mutates ledger state.

use serde::Serialize;

/// Normalized command-center status bucket.
///
/// Collapses the ledger's eight `AgentRunStatus` values into the five groups
/// the command center renders. The mapping lives in
/// [`super::ops::bucket_for`]; the order of [`AgentWorkBucket::ALL`] is the
/// display order (needs-input first so blocked work is most visible).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentWorkBucket {
    /// A run paused waiting for user input (`awaiting_user`).
    NeedsInput,
    /// A run still executing or queued (`pending` / `running` / `paused`).
    Working,
    /// A run that finished successfully (`completed`).
    Completed,
    /// A run that ended in error (`failed`).
    Failed,
    /// A run cancelled or interrupted before completion.
    Stopped,
}

impl AgentWorkBucket {
    /// Display order: needs-input first, then in-flight, then terminal states.
    pub const ALL: [AgentWorkBucket; 5] = [
        AgentWorkBucket::NeedsInput,
        AgentWorkBucket::Working,
        AgentWorkBucket::Completed,
        AgentWorkBucket::Failed,
        AgentWorkBucket::Stopped,
    ];

    /// Stable wire string for this bucket.
    pub fn as_str(self) -> &'static str {
        match self {
            AgentWorkBucket::NeedsInput => "needs_input",
            AgentWorkBucket::Working => "working",
            AgentWorkBucket::Completed => "completed",
            AgentWorkBucket::Failed => "failed",
            AgentWorkBucket::Stopped => "stopped",
        }
    }
}

/// One command-center row, projected from a durable [`AgentRun`].
///
/// Kept deliberately lean — transcripts and checkpoints stay in the ledger /
/// thread stores and are fetched on demand when a user opens a row.
///
/// [`AgentRun`]: crate::openhuman::session_db::run_ledger::AgentRun
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentWorkRow {
    /// Ledger run id (open/peek key).
    pub run_id: String,
    /// Run kind wire string (`subagent` / `worker_thread` / ...).
    pub kind: String,
    /// Agent definition id, when known.
    pub agent_id: Option<String>,
    /// Human-friendly name resolved from the agent registry, when available.
    pub display_name: Option<String>,
    /// Normalized command-center bucket.
    pub bucket: AgentWorkBucket,
    /// Raw ledger status wire string (preserved for detail views).
    pub status: String,
    /// Parent conversation thread, for deterministic "open thread".
    pub parent_thread_id: Option<String>,
    /// Linked worker thread, for "open worker transcript".
    pub worker_thread_id: Option<String>,
    /// Latest summary, when the run produced one.
    pub summary: Option<String>,
    /// Failure reason, when the run failed.
    pub error: Option<String>,
    /// RFC3339 start timestamp.
    pub started_at: String,
    /// RFC3339 last-activity timestamp.
    pub updated_at: String,
    /// Wall-clock elapsed milliseconds, when telemetry recorded it.
    pub elapsed_ms: Option<u64>,
    /// Input tokens spent (0 when no telemetry).
    pub input_tokens: u64,
    /// Output tokens spent (0 when no telemetry).
    pub output_tokens: u64,
    /// Cost in USD (0 when no telemetry).
    pub cost_usd: f64,
    /// Tool-call count (0 when no telemetry).
    pub tool_count: u64,
}

/// One status group in the command-center view.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandCenterGroup {
    /// The bucket this group represents.
    pub bucket: AgentWorkBucket,
    /// Number of rows in this group.
    pub count: usize,
    /// Rows, most-recently-updated first.
    pub rows: Vec<AgentWorkRow>,
}

/// The full command-center view: all five buckets in display order plus a total.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandCenterView {
    /// Always exactly five groups, in [`AgentWorkBucket::ALL`] order (empty
    /// groups included so the UI can render stable section headers).
    pub groups: Vec<CommandCenterGroup>,
    /// Total rows across all buckets.
    pub total: usize,
}
