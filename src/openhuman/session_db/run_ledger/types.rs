use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunKind {
    Subagent,
    WorkerThread,
    BackgroundAgent,
    TeamMember,
    WorkflowChild,
}

impl AgentRunKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Subagent => "subagent",
            Self::WorkerThread => "worker_thread",
            Self::BackgroundAgent => "background_agent",
            Self::TeamMember => "team_member",
            Self::WorkflowChild => "workflow_child",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw {
            "worker_thread" => Self::WorkerThread,
            "background_agent" => Self::BackgroundAgent,
            "team_member" => Self::TeamMember,
            "workflow_child" => Self::WorkflowChild,
            _ => Self::Subagent,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunStatus {
    Pending,
    Running,
    AwaitingUser,
    Paused,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl AgentRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::AwaitingUser => "awaiting_user",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw {
            "running" => Self::Running,
            "awaiting_user" => Self::AwaitingUser,
            "paused" => Self::Paused,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "interrupted" => Self::Interrupted,
            _ => Self::Pending,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRunStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl WorkflowRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw {
            "running" => Self::Running,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "interrupted" => Self::Interrupted,
            _ => Self::Pending,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRun {
    pub id: String,
    pub kind: AgentRunKind,
    pub parent_run_id: Option<String>,
    pub parent_thread_id: Option<String>,
    pub agent_id: Option<String>,
    pub status: AgentRunStatus,
    pub prompt_ref: Option<String>,
    pub worker_thread_id: Option<String>,
    pub task_board_id: Option<String>,
    pub task_card_id: Option<String>,
    pub checkpoint_path: Option<String>,
    pub checkpoint: Option<Value>,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub metadata: Value,
    pub telemetry: Option<RunTelemetry>,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRun {
    pub id: String,
    pub definition_id: String,
    pub parent_thread_id: Option<String>,
    pub input: Value,
    pub phase_states: Value,
    pub child_run_ids: Vec<String>,
    pub status: WorkflowRunStatus,
    pub summary: Option<String>,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEvent {
    pub run_id: String,
    pub sequence: u64,
    pub event_type: String,
    pub payload: Value,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RunTelemetry {
    pub run_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub cost_usd: f64,
    pub elapsed_ms: Option<u64>,
    pub tool_count: u64,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub error: Option<String>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct AgentRunUpsert {
    pub id: String,
    pub kind: AgentRunKind,
    pub parent_run_id: Option<String>,
    pub parent_thread_id: Option<String>,
    pub agent_id: Option<String>,
    pub status: AgentRunStatus,
    pub prompt_ref: Option<String>,
    pub worker_thread_id: Option<String>,
    pub task_board_id: Option<String>,
    pub task_card_id: Option<String>,
    pub checkpoint_path: Option<String>,
    pub checkpoint: Option<Value>,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub metadata: Value,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct WorkflowRunUpsert {
    pub id: String,
    pub definition_id: String,
    pub parent_thread_id: Option<String>,
    pub input: Value,
    pub phase_states: Value,
    pub child_run_ids: Vec<String>,
    pub status: WorkflowRunStatus,
    pub summary: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct RunEventAppend {
    pub run_id: String,
    pub event_type: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Default)]
pub struct RunTelemetryUpsert {
    pub run_id: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    pub elapsed_ms: Option<u64>,
    pub tool_count: Option<u64>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunListRequest {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub parent_run_id: Option<String>,
    #[serde(default)]
    pub parent_thread_id: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunListResponse {
    pub runs: Vec<AgentRun>,
    pub count: usize,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEventListRequest {
    pub run_id: String,
    #[serde(default)]
    pub after_sequence: Option<u64>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEventListResponse {
    pub events: Vec<RunEvent>,
    pub count: usize,
}
