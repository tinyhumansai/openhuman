use serde::{Deserialize, Serialize};

/// Decision produced by the subconscious tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Nothing meaningful changed — skip.
    Noop,
    /// Something changed that can be handled locally (store memory, update state).
    Act,
    /// Something important changed that requires the full agent.
    Escalate,
}

/// A single action recommended by the subconscious.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendedAction {
    #[serde(rename = "type")]
    pub action_type: ActionType,
    pub description: String,
    pub priority: Priority,
    /// Which HEARTBEAT.md task this action relates to.
    #[serde(default)]
    pub task: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    Notify,
    StoreMemory,
    EscalateToAgent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    Medium,
    High,
}

/// Output from the local model after evaluating the situation report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TickOutput {
    pub decision: Decision,
    pub reason: String,
    #[serde(default)]
    pub actions: Vec<RecommendedAction>,
}

/// Result of a single subconscious tick, including metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TickResult {
    pub tick_at: f64,
    pub output: TickOutput,
    pub source_doc_ids: Vec<String>,
    pub duration_ms: u64,
    pub tokens_used: u64,
}

/// Summary of the subconscious loop status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubconsciousStatus {
    pub enabled: bool,
    pub interval_minutes: u32,
    pub last_tick_at: Option<f64>,
    pub last_decision: Option<Decision>,
    pub total_ticks: u64,
    pub total_escalations: u64,
}
