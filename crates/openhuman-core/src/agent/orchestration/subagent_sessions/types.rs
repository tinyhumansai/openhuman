use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::agent::subagent_host::SubagentRunStatus;
use tinyagents_session::transcript::TranscriptMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurableSubagentStatus {
    Running,
    Idle,
    AwaitingUser,
    Failed,
    Closed,
}

impl DurableSubagentStatus {
    pub fn from_run_status(status: &SubagentRunStatus) -> Self {
        match status {
            SubagentRunStatus::Completed => Self::Idle,
            SubagentRunStatus::AwaitingUser { .. } => Self::AwaitingUser,
            // Stopped short (stuck halt / iteration cap). The run is no longer
            // active and can be resumed/reused, so it maps to Idle — not Failed
            // (no hard error) and not AwaitingUser (not paused for input).
            SubagentRunStatus::Incomplete { .. } => Self::Idle,
            SubagentRunStatus::Cancelled => Self::Failed,
        }
    }

    pub fn reusable(self) -> bool {
        matches!(self, Self::Running | Self::Idle | Self::AwaitingUser)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentSessionSelector {
    pub parent_session: String,
    pub parent_thread_id: Option<String>,
    pub agent_id: String,
    pub model: Option<String>,
    pub sandbox_mode: String,
    pub action_root: Option<String>,
    pub task_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DurableSubagentSession {
    pub subagent_session_id: String,
    pub parent_session: String,
    pub parent_thread_id: Option<String>,
    pub worker_thread_id: Option<String>,
    pub agent_id: String,
    pub display_name: Option<String>,
    pub model: Option<String>,
    pub sandbox_mode: String,
    pub action_root: Option<String>,
    pub task_key: String,
    pub task_title: String,
    pub current_task_id: Option<String>,
    /// TinyAgents' full lifecycle identity for the current task. Older rows
    /// do not contain this field; hosts recover those identities from the
    /// validated checkpoint before resuming or controlling a live child.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle_key: Option<tinyagents_orchestration::subagent::SubagentTaskKey>,
    /// Parent lifecycle generation admitted for `current_task_id`. Older rows
    /// intentionally remain `None`; callers must never synthesize a new epoch
    /// when settling a legacy paused child.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_epoch: Option<u64>,
    pub status: DurableSubagentStatus,
    pub reusable: bool,
    #[serde(default, with = "crate::agent::messages::history_wire::option")]
    pub latest_history: Option<Vec<TranscriptMessage>>,
    pub latest_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_used_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DurableSubagentSessionSummary {
    pub subagent_session_id: String,
    pub parent_thread_id: Option<String>,
    pub worker_thread_id: Option<String>,
    pub agent_id: String,
    pub display_name: Option<String>,
    pub model: Option<String>,
    pub sandbox_mode: String,
    pub action_root: Option<String>,
    pub task_key: String,
    pub task_title: String,
    pub current_task_id: Option<String>,
    pub status: DurableSubagentStatus,
    pub reusable: bool,
    pub latest_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_used_at: String,
}

impl From<&DurableSubagentSession> for DurableSubagentSessionSummary {
    fn from(session: &DurableSubagentSession) -> Self {
        Self {
            subagent_session_id: session.subagent_session_id.clone(),
            parent_thread_id: session.parent_thread_id.clone(),
            worker_thread_id: session.worker_thread_id.clone(),
            agent_id: session.agent_id.clone(),
            display_name: session.display_name.clone(),
            model: session.model.clone(),
            sandbox_mode: session.sandbox_mode.clone(),
            action_root: session.action_root.clone(),
            task_key: session.task_key.clone(),
            task_title: session.task_title.clone(),
            current_task_id: session.current_task_id.clone(),
            status: session.status,
            reusable: session.reusable,
            latest_error: session.latest_error.clone(),
            created_at: session.created_at.clone(),
            updated_at: session.updated_at.clone(),
            last_used_at: session.last_used_at.clone(),
        }
    }
}

impl DurableSubagentSession {
    pub fn matches_selector(&self, selector: &SubagentSessionSelector) -> bool {
        self.reusable
            && self.status.reusable()
            && self.parent_session == selector.parent_session
            && self.parent_thread_id == selector.parent_thread_id
            && self.agent_id == selector.agent_id
            && self.model == selector.model
            && self.sandbox_mode == selector.sandbox_mode
            && self.action_root == selector.action_root
            && self.task_key == selector.task_key
    }
}

#[derive(Debug, Clone)]
pub struct SubagentSessionUpsert {
    pub selector: SubagentSessionSelector,
    pub display_name: Option<String>,
    pub task_title: String,
    pub worker_thread_id: Option<String>,
    pub task_id: String,
    /// Full TinyAgents lifecycle identity when the caller owns one. `None` is
    /// retained for legacy host paths and rows written before this field was
    /// introduced.
    pub lifecycle_key: Option<tinyagents_orchestration::subagent::SubagentTaskKey>,
    /// Admission generation captured for this exact current task.
    pub parent_epoch: Option<u64>,
}

#[derive(Clone)]
pub struct SubagentSessionStore {
    pub workspace_dir: PathBuf,
    pub(crate) context: Option<std::sync::Arc<crate::core::runtime::CoreContext>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReuseDecision {
    ReusedRunning,
    ReusedIdle,
    SpawnedNew,
    ForcedFresh,
}

impl ReuseDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ReusedRunning => "reused_running",
            Self::ReusedIdle => "reused_idle",
            Self::SpawnedNew => "spawned_new",
            Self::ForcedFresh => "forced_fresh",
        }
    }
}
