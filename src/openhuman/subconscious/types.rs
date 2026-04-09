//! Type definitions for the subconscious task execution system.

use serde::{Deserialize, Serialize};

// ── Task types ───────────────────────────────────────────────────────────────

/// A task managed by the subconscious engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubconsciousTask {
    pub id: String,
    pub title: String,
    pub source: TaskSource,
    pub recurrence: TaskRecurrence,
    pub enabled: bool,
    pub last_run_at: Option<f64>,
    pub next_run_at: Option<f64>,
    pub completed: bool,
    pub created_at: f64,
}

/// Where the task came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskSource {
    /// Auto-populated by the system (skills health, Ollama status, etc.)
    System,
    /// Added by the user via UI or agent.
    User,
}

/// How often the task should run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskRecurrence {
    /// Execute once, then mark completed.
    Once,
    /// Recurrent on a cron schedule (5-field expression).
    Cron(String),
    /// Not yet classified — agent will decide on first tick.
    Pending,
}

/// Partial update for a task.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub recurrence: Option<TaskRecurrence>,
    pub enabled: Option<bool>,
}

// ── Tick evaluation types ────────────────────────────────────────────────────

/// Per-tick decision for a single task.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TickDecision {
    /// Nothing relevant in current state for this task.
    #[default]
    Noop,
    /// State has something relevant — execute the task.
    Act,
    /// Ambiguous or risky — surface to user for approval.
    Escalate,
}

/// The local model's evaluation of a single task against the current state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskEvaluation {
    pub task_id: String,
    pub decision: TickDecision,
    pub reason: String,
}

/// Full evaluation response from the local model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationResponse {
    pub evaluations: Vec<TaskEvaluation>,
}

// ── Execution types ──────────────────────────────────────────────────────────

/// Result of executing a single task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub output: String,
    pub used_tools: bool,
    pub duration_ms: u64,
}

// ── Log types ────────────────────────────────────────────────────────────────

/// A single entry in the execution log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubconsciousLogEntry {
    pub id: String,
    pub task_id: String,
    pub tick_at: f64,
    pub decision: String,
    pub result: Option<String>,
    pub duration_ms: Option<i64>,
    pub created_at: f64,
}

// ── Escalation types ─────────────────────────────────────────────────────────

/// An escalation waiting for user input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Escalation {
    pub id: String,
    pub task_id: String,
    pub log_id: Option<String>,
    pub title: String,
    pub description: String,
    pub priority: EscalationPriority,
    pub status: EscalationStatus,
    pub created_at: f64,
    pub resolved_at: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EscalationPriority {
    Critical,
    Important,
    Normal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EscalationStatus {
    Pending,
    Approved,
    Dismissed,
}

// ── Status types ─────────────────────────────────────────────────────────────

/// Summary of the subconscious engine status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubconsciousStatus {
    pub enabled: bool,
    pub interval_minutes: u32,
    pub last_tick_at: Option<f64>,
    pub total_ticks: u64,
    pub task_count: u64,
    pub pending_escalations: u64,
    /// Number of consecutive tick failures (resets on success).
    pub consecutive_failures: u64,
}

/// Result of a single subconscious tick.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TickResult {
    pub tick_at: f64,
    pub evaluations: Vec<TaskEvaluation>,
    pub executed: usize,
    pub escalated: usize,
    pub duration_ms: u64,
}
