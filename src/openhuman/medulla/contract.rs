//! Serde data types for the public agent-harness wire shapes. Field renames pin
//! every struct to the JSON contract
//! (`rename_all = "camelCase"`); enum renames pin the lowercase status/state
//! strings used on the wire. Only shapes and their trivial impls live here —
//! the reserved tool-name vocabulary and re-exports live in the parent module.
//!
//! Two payloads are intentionally opaque: `HarnessStatus::last_result` and
//! `HarnessEvent::CycleEvent { event }`. Neither payload is a contract this
//! crate consumes, so both
//! are kept as [`serde_json::Value`] rather than mirrored field-by-field; this
//! preserves them losslessly without coupling the client to their internals.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Lifecycle of a tracked task. The lowercase rename matches the wire strings
/// (`"open"`, `"active"`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackedTaskStatus {
    /// Opened, not yet started.
    Open,
    /// Actively being worked.
    Active,
    /// Blocked on something.
    Blocked,
    /// Completed.
    Done,
    /// Abandoned.
    Cancelled,
}

/// Advisory boundaries and completion criteria for one delegated worker lane.
/// Medulla transports this object verbatim; enforcement remains host-owned.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerContract {
    /// The exact outcome the lane should produce.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// Workspace globs the lane is permitted to change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permitted_paths: Option<Vec<String>>,
    /// Work explicitly excluded from this lane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub non_goals: Option<Vec<String>>,
    /// Focused verification commands the worker should run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify_commands: Option<Vec<String>>,
    /// Condition that ends the lane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_condition: Option<String>,
}

/// One command or named gate outcome attributed to a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationEvidence {
    /// Exact verification command, mutually exclusive with `gate` by convention.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Named library gate, mutually exclusive with `command` by convention.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<String>,
    /// Whether the verification passed.
    pub ok: bool,
    /// Bounded human-readable outcome.
    pub summary: String,
}

/// One unit of intended work on the session task board.
/// `created_at`/`updated_at` are ISO-8601 strings and
/// `delegated_task_ids`/`notes` are always-present arrays (never omitted).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackedTask {
    /// Stable task id (minted host-side).
    pub id: String,
    /// Short imperative title.
    pub title: String,
    /// Optional longer description; omitted from JSON when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Current lifecycle status.
    pub status: TrackedTaskStatus,
    /// ISO-8601 creation timestamp.
    pub created_at: String,
    /// ISO-8601 timestamp of the last update.
    pub updated_at: String,
    /// The originating orchestrator instruction / cycle, when recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction_id: Option<String>,
    /// Delegation-ledger keys this task fanned out to.
    #[serde(default)]
    pub delegated_task_ids: Vec<String>,
    /// Append-only free-form notes.
    #[serde(default)]
    pub notes: Vec<String>,
    /// Worker-lane boundaries associated with a linked delegation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<WorkerContract>,
    /// Append-only per-task verification observations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Vec<VerificationEvidence>>,
}

/// The run state of the agent harness. The lowercase rename matches the wire
/// strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HarnessState {
    /// No instruction is running and the queue is empty.
    Idle,
    /// A cycle is running or the queue is non-empty.
    Running,
    /// `stop()` has latched; no new work starts.
    Stopped,
}

/// Rolled-up token/cycle accounting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessUsage {
    /// Completed cycles.
    pub cycles: u64,
    /// Total prompt tokens across cycles and settled delegations.
    pub input_tokens: u64,
    /// Total completion tokens across cycles and settled delegations.
    pub output_tokens: u64,
}

/// A synchronous snapshot of the agent harness.
///
/// `last_result` is kept opaque (see the module doc): present after the first
/// cycle settles, omitted before then.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessStatus {
    /// Run state.
    pub state: HarnessState,
    /// Instructions still waiting in the FIFO queue.
    pub queued: u64,
    /// The running instruction's id, when a cycle is active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_instruction_id: Option<String>,
    /// The running cycle's id, when a cycle is active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_cycle_id: Option<String>,
    /// The live task board.
    #[serde(default)]
    pub tasks: Vec<TrackedTask>,
    /// Currently in-flight delegated sub-agent tasks.
    pub running_delegations: u64,
    /// Rolled-up usage.
    pub usage: HarnessUsage,
    /// The most recent cycle result (opaque `CycleResult`); omitted before the
    /// first cycle settles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_result: Option<Value>,
    /// Escalation messages accumulated across cycles.
    #[serde(default)]
    pub escalations: Vec<String>,
}

/// A harness observability event, tagged by its `kind` field. The three
/// lifecycle kinds share a shape but stay distinct
/// variants so the `kind` string round-trips exactly.
///
/// `CycleEvent { event }` keeps its inner event opaque (see the module doc).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// Keep the public `task: TrackedTask` field source-compatible; boxing only this
// wire variant would impose an unrelated API migration on an additive schema.
#[allow(clippy::large_enum_variant)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum HarnessEvent {
    /// An instruction was enqueued.
    InstructionQueued {
        /// The enqueued instruction's id.
        instruction_id: String,
        /// The cycle id minted for it.
        cycle_id: String,
    },
    /// A cycle started.
    CycleStart {
        /// The running instruction's id.
        instruction_id: String,
        /// The running cycle's id.
        cycle_id: String,
    },
    /// A cycle ended.
    CycleEnd {
        /// The finished instruction's id.
        instruction_id: String,
        /// The finished cycle's id.
        cycle_id: String,
    },
    /// A task board entry changed.
    TaskBoardChanged {
        /// The task whose state changed.
        task: TrackedTask,
    },
    /// A raw per-cycle event was re-emitted (opaque `CycleEvent`).
    CycleEvent {
        /// The wrapped cycle event payload.
        event: Value,
    },
}

/// The serialisable receipt returned when an instruction is queued.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstructionReceipt {
    /// The enqueued instruction's id.
    pub instruction_id: String,
    /// The cycle id minted for it.
    pub cycle_id: String,
}

/// Machine-readable seat attribution stamped onto an agent descriptor at
/// `metadata.budget`.
///
/// Note the timestamp contrast with [`SeatHeadroom`]: here `primary_resets_at`
/// is an **ISO-8601 string** (formatted at the roster boundary), whereas
/// `SeatHeadroom` carries epoch-milliseconds numbers throughout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBudgetMetadata {
    /// The seat this agent draws from.
    pub seat_id: String,
    /// The metering provider (e.g. `"anthropic"`, `"openai"`).
    pub provider: String,
    /// The plan id (e.g. `"claude_max_5x"`).
    pub plan: String,
    /// The human-facing plan label (e.g. `"Claude Max 5×"`).
    pub plan_label: String,
    /// Remaining headroom, in tokens, at snapshot time.
    pub headroom_tokens: u64,
    /// True when headroom is below the exhausted floor.
    pub exhausted: bool,
    /// ISO-8601 timestamp the primary window next refills.
    pub primary_resets_at: String,
}

/// Remaining tokens and next-reset for one refill window, keyed by window id in
/// [`SeatHeadroom::per_window`]. Times are epoch-milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowHeadroom {
    /// Tokens left in this window.
    pub remaining: u64,
    /// Epoch-ms this window next refills.
    pub resets_at: i64,
}

/// Live headroom for one connected seat. All timestamps are epoch-milliseconds
/// numbers (contrast
/// [`AgentBudgetMetadata`], which formats `primary_resets_at` to an ISO string).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeatHeadroom {
    /// Stable seat id.
    pub seat_id: String,
    /// The metering provider.
    pub provider: String,
    /// The plan id resolved against the plan catalog.
    pub plan: String,
    /// The human-facing plan label.
    pub plan_label: String,
    /// Roster agent ids this seat is pinned to.
    #[serde(default)]
    pub agent_ids: Vec<String>,
    /// Whether the seat is enabled for delegation.
    pub enabled: bool,
    /// Lower wins when picking among a provider's seats.
    pub priority: i64,
    /// Min headroom across windows; 0 when disabled or throttled.
    pub headroom_tokens: u64,
    /// True when headroom is below the exhausted floor.
    pub exhausted: bool,
    /// Epoch-ms the seat is throttled until; omitted when not throttled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub throttled_until: Option<i64>,
    /// Epoch-ms the primary (first) window next refills.
    pub primary_resets_at: i64,
    /// Remaining tokens + next-reset per window, keyed by window id.
    #[serde(default)]
    pub per_window: BTreeMap<String, WindowHeadroom>,
}

impl AgentBudgetMetadata {
    /// Parse the budget stamp out of an agent descriptor's `metadata` map, if a
    /// well-formed `budget` object is present. Returns `None` when the key is
    /// absent or the value does not match the [`AgentBudgetMetadata`] shape, so a
    /// malformed stamp degrades to "no budget shown" rather than erroring.
    pub fn from_metadata(metadata: &serde_json::Map<String, Value>) -> Option<AgentBudgetMetadata> {
        let budget = metadata.get("budget")?;
        serde_json::from_value(budget.clone()).ok()
    }
}
