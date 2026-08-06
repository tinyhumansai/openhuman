//! Wire models for the worker roster and operator-owned task program APIs.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Client-safe harness budget advertised by a connected worker.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RosterBudget {
    /// Provider that meters the budget.
    pub provider: String,
    /// Provider-defined accounting window.
    pub window: String,
    /// Tokens still available in the current window, when known.
    #[serde(default)]
    pub remaining_tokens: Option<u64>,
    /// Total token allowance for the current window, when known.
    #[serde(default)]
    pub limit_tokens: Option<u64>,
    /// Unix timestamp after which a depleted budget becomes usable.
    #[serde(default)]
    pub cooldown_until: Option<u64>,
    /// Origin of the budget observation.
    pub source: String,
}

/// Connected worker returned by `GET /medulla/v1/roster`.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RosterWorker {
    /// Stable worker identifier in the manager registry.
    pub registry_id: String,
    /// Operator-facing worker label.
    pub label: String,
    /// Operator-facing description of the worker.
    pub description: String,
    /// Current manager-reported availability.
    pub availability: String,
    /// Harness implementation serving the worker.
    #[serde(default)]
    pub harness: Option<String>,
    /// Network address advertised by the worker.
    #[serde(default)]
    pub address: Option<String>,
    /// Provider-specific worker handle.
    #[serde(default)]
    pub handle: Option<String>,
    /// Peer identifier used for wallet-backed routing.
    #[serde(default)]
    pub wallet_peer_id: Option<String>,
    /// Logical CPU capacity visible to the worker.
    #[serde(default)]
    pub cpu_cores: Option<u32>,
    /// Total host memory in bytes.
    #[serde(default)]
    pub total_mem_bytes: Option<u64>,
    /// Currently available host memory in bytes.
    #[serde(default)]
    pub available_mem_bytes: Option<u64>,
    /// Primary IPv4 address selected by the manager.
    #[serde(default)]
    pub primary_ipv4: Option<String>,
    /// Routing and discovery labels attached to the worker.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Whether the worker is currently selected for dispatch.
    pub selected: bool,
    /// Provider-specific capability document.
    #[serde(default)]
    pub capabilities: Option<Value>,
    /// Client-safe budgets reported for this worker.
    #[serde(default)]
    pub budgets: Vec<RosterBudget>,
}

/// Response payload of `GET /medulla/v1/roster`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Roster {
    /// Workers currently known to the manager.
    #[serde(default)]
    pub workers: Vec<RosterWorker>,
}

/// Lifecycle state of an operator-owned program task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProgramTaskStatus {
    /// Ready for work.
    Open,
    /// Work has started.
    InProgress,
    /// Work completed successfully.
    Done,
    /// Work was intentionally abandoned.
    Cancelled,
}

/// Recurrence frequency for an operator-owned program task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskRecurrenceFrequency {
    /// Repeat every day.
    Daily,
    /// Repeat every week.
    Weekly,
    /// Repeat every month.
    Monthly,
    /// Repeat after a custom number of days.
    EveryDays,
}

/// Optional recurrence rule attached to a program task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecurrence {
    /// Schedule family used to calculate the next occurrence.
    pub frequency: TaskRecurrenceFrequency,
    /// Custom day interval used with [`TaskRecurrenceFrequency::EveryDays`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_days: Option<u32>,
    /// RFC 3339 timestamp for the next scheduled occurrence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_at: Option<String>,
}

/// External source identity attached to a synchronized task.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramTaskSourceRef {
    /// Source provider name.
    pub provider: String,
    /// Stable item identifier assigned by the provider.
    pub source_id: String,
    /// Browser URL for the source item.
    #[serde(default)]
    pub url: Option<String>,
}

/// An operator-owned task in the backend program ledger.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramTask {
    /// Stable backend task identifier.
    pub id: String,
    /// Short operator-facing title.
    pub title: String,
    /// Detailed task description.
    pub description: String,
    /// Current task lifecycle state.
    pub status: ProgramTaskStatus,
    /// Recurrence rule, if the task repeats.
    #[serde(default)]
    pub recurrence: Option<TaskRecurrence>,
    /// External item that produced this task.
    #[serde(default)]
    pub source: Option<ProgramTaskSourceRef>,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
    /// RFC 3339 timestamp of the latest update.
    pub updated_at: String,
    /// RFC 3339 timestamp of the latest source synchronization.
    #[serde(default)]
    pub last_synced_at: Option<String>,
    /// Backend-owned dispatch state.
    #[serde(default)]
    pub dispatch: Value,
}

/// Input for creating a program task.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProgramTask {
    /// Short operator-facing title.
    pub title: String,
    /// Detailed task description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Initial lifecycle state; the backend default applies when omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ProgramTaskStatus>,
    /// Initial recurrence rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recurrence: Option<TaskRecurrence>,
}

/// Patch accepted by `PATCH /medulla/v1/tasks/:id`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgramTask {
    /// Replacement title, or omission to preserve the current title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Replacement description, or omission to preserve the current description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Replacement lifecycle state, or omission to preserve the current state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ProgramTaskStatus>,
    /// Recurrence patch: outer `None` omits the field, inner `None` sends JSON
    /// `null` to clear recurrence, and `Some(Some(_))` replaces the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recurrence: Option<Option<TaskRecurrence>>,
}

/// GitHub issue state selected by a task source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GithubIssueState {
    /// Synchronize open issues.
    Open,
    /// Synchronize closed issues.
    Closed,
    /// Synchronize issues in either state.
    All,
}

/// A configured GitHub task source. Tokens never appear on this response.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramTaskSource {
    /// Stable backend source identifier.
    pub id: String,
    /// Source provider name.
    pub provider: String,
    /// Whether periodic synchronization is enabled.
    pub enabled: bool,
    /// GitHub repository in `owner/name` form.
    pub repository: String,
    /// Issue state included by the source.
    pub state: GithubIssueState,
    /// Labels that issues must match.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Additional provider-defined search filter.
    #[serde(default)]
    pub filter: Option<String>,
    /// Whether a provider token is configured; the token itself is never returned.
    pub has_token: bool,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
    /// RFC 3339 timestamp of the latest update.
    pub updated_at: String,
}

/// Input for configuring a GitHub task source.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProgramTaskSource {
    /// GitHub repository in `owner/name` form.
    pub repository: String,
    /// Issue state to synchronize.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<GithubIssueState>,
    /// Labels that issues must match.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<Vec<String>>,
    /// Additional provider-defined search filter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    /// Provider token to store; this write-only value is never returned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Whether periodic synchronization should be enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

/// Result of synchronizing one GitHub source into the task ledger.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TaskSourceSyncResult {
    /// Number of tasks created.
    pub added: u64,
    /// Number of existing tasks updated.
    pub updated: u64,
    /// Number of source items already current.
    pub unchanged: u64,
    /// Non-fatal provider errors encountered during synchronization.
    #[serde(default)]
    pub errors: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct TasksPayload {
    pub(crate) tasks: Vec<ProgramTask>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct TaskPayload {
    pub(crate) task: ProgramTask,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct TaskSourcesPayload {
    pub(crate) sources: Vec<ProgramTaskSource>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct TaskSourcePayload {
    pub(crate) source: ProgramTaskSource,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct TaskSourceSyncPayload {
    pub(crate) result: TaskSourceSyncResult,
}

/// Result shared by program task/source delete endpoints.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeleteProgramItem {
    /// Whether the requested resource existed and was removed.
    pub deleted: bool,
}
