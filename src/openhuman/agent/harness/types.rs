//! Shared types for the multi-agent harness: requests, results, task status.

use super::archetypes::AgentArchetype;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Opaque identifier for a task node inside a `TaskDag`.
pub type TaskId = String;

/// Current execution status of a DAG task node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl Default for TaskStatus {
    fn default() -> Self {
        Self::Pending
    }
}

/// Request sent from the orchestrator to spawn a sub-agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentRequest {
    /// Task node this request fulfils.
    pub task_id: TaskId,
    /// Which archetype to instantiate.
    pub archetype: AgentArchetype,
    /// The specific instruction for this sub-agent.
    pub prompt: String,
    /// Optional context chunks injected before the prompt.
    #[serde(default)]
    pub context: Vec<ContextChunk>,
    /// Parent session id (for cost aggregation and memory scoping).
    pub parent_session_id: String,
    /// Maximum wall-clock time for this sub-agent.
    #[serde(
        default = "default_subagent_timeout",
        with = "humantime_serde",
        skip_serializing_if = "is_default_timeout"
    )]
    pub timeout: Duration,
}

fn default_subagent_timeout() -> Duration {
    Duration::from_secs(120)
}

fn is_default_timeout(d: &Duration) -> bool {
    *d == default_subagent_timeout()
}

/// A labelled piece of context forwarded to a sub-agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextChunk {
    pub label: String,
    pub content: String,
}

/// Result returned by a completed (or failed) sub-agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentResult {
    pub task_id: TaskId,
    pub success: bool,
    pub output: String,
    /// File paths or diffs produced by this agent.
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    /// Cost in micro-dollars (1 USD = 1_000_000).
    #[serde(default)]
    pub cost_microdollars: u64,
    /// Wall-clock duration of the sub-agent run.
    #[serde(default, with = "humantime_serde")]
    pub duration: Duration,
}

/// An artifact produced by a sub-agent (file written, diff, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub kind: ArtifactKind,
    pub path: Option<String>,
    pub content: String,
}

/// Classification of artifacts produced by sub-agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    FileWritten,
    Diff,
    TestResult,
    LintResult,
    Summary,
    Other,
}

/// Decision the orchestrator makes after reviewing a completed DAG level.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    /// Continue to the next DAG level.
    Continue,
    /// Retry specific failed tasks (up to limit).
    Retry(Vec<TaskId>),
    /// Abort the entire DAG with a reason.
    Abort(String),
}

// Provide a minimal humantime_serde so we can skip adding the crate as a dep.
// If humantime_serde is already available, swap these out.
mod humantime_serde {
    use serde::{self, Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_f64(duration.as_secs_f64())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = f64::deserialize(deserializer)?;
        Ok(Duration::from_secs_f64(secs))
    }
}
