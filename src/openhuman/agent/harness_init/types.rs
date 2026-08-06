//! Serde types for the harness-init status snapshot.
//!
//! These are the wire shapes returned by `openhuman.harness_init_status` /
//! `openhuman.harness_init_run` and consumed by the frontend initialization
//! screen. All enums serialize `snake_case` so the TypeScript side can match
//! on plain string literals.

use serde::{Deserialize, Serialize};

/// State of a single init step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    /// Not started yet.
    Pending,
    /// Work in progress.
    Running,
    /// Completed successfully (or already provisioned).
    Done,
    /// A required step failed.
    Failed,
    /// A non-required step failed; the app proceeds with a fallback.
    Skipped,
}

impl StepState {
    /// A step in `Done`/`Failed`/`Skipped` will not change again this run.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Skipped)
    }
}

/// Per-step status surfaced to the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepStatus {
    /// Stable identifier, e.g. `"python_runtime"`.
    pub id: String,
    /// Human-readable label (the UI may prefer its own i18n key by `id`).
    pub label: String,
    /// Whether a failure of this step blocks the app. All steps are currently
    /// non-required (their absence degrades to a fallback).
    pub required: bool,
    /// Current lifecycle state.
    pub state: StepState,
    /// Optional detail (error string, "already provisioned", etc.).
    pub message: Option<String>,
    /// Optional 0–100 progress hint; `None` for indeterminate steps.
    pub percent: Option<u8>,
    /// RFC3339 timestamp of the last state change.
    pub updated_at: Option<String>,
}

/// Overall init lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverallState {
    /// No run has started.
    Idle,
    /// At least one step is pending/running.
    Running,
    /// All steps reached a terminal state and no required step failed.
    Done,
    /// A required step failed.
    Failed,
}

impl OverallState {
    /// The UI stops polling / unblocks once the run is terminal.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed)
    }
}

/// Full snapshot returned over RPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessInitSnapshot {
    pub overall: OverallState,
    pub steps: Vec<StepStatus>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}
