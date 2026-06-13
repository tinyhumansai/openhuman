//! Public data model for high-level agent orchestration.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Stable status vocabulary for parent/child orchestration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Pending,
    Running,
    Waiting,
    Completed,
    Failed,
    Cancelled,
    Closed,
}

impl AgentStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Closed
        )
    }
}

/// Message recorded on an orchestration child.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMessage {
    pub role: String,
    pub content: String,
    pub created_at: String,
}

/// Request to spawn a child agent from the current parent agent turn.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpawnAgentRequest {
    pub agent_id: String,
    pub prompt: String,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub toolkit: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub parent_agent_id: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

/// Result returned immediately after an agent is accepted for execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnAgentResponse {
    pub orchestration_id: String,
    pub agent_id: String,
    pub status: AgentStatus,
}

/// Request to append a parent/child message to a child agent record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageAgentRequest {
    pub orchestration_id: String,
    pub content: String,
}

/// Request to close or cancel a child agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloseAgentRequest {
    pub orchestration_id: String,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Follow-up work for an existing child. Follow-ups spawn a new child linked
/// to the previous child instead of mutating a completed transcript in place.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FollowUpRequest {
    pub orchestration_id: String,
    pub prompt: String,
    #[serde(default)]
    pub context: Option<String>,
}

/// Resume a previous child by spawning a linked continuation child.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeAgentRequest {
    pub orchestration_id: String,
    #[serde(default)]
    pub prompt: Option<String>,
}

/// Wait request for one or more children.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WaitAgentOptions {
    pub orchestration_ids: Vec<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// Wait response with the latest snapshots known to the orchestrator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitAgentResponse {
    pub completed: bool,
    pub agents: Vec<AgentSnapshot>,
}

/// Serializable child-agent state for UI, diagnostics, and future persistence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSnapshot {
    pub orchestration_id: String,
    pub agent_id: String,
    pub parent_agent_id: Option<String>,
    pub status: AgentStatus,
    pub prompt: String,
    pub messages: Vec<AgentMessage>,
    pub result_summary: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

/// Lifecycle events emitted by the high-level orchestration domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentOrchestrationEvent {
    Spawned {
        orchestration_id: String,
        agent_id: String,
        parent_agent_id: Option<String>,
    },
    MessageRecorded {
        orchestration_id: String,
    },
    Completed {
        orchestration_id: String,
        output_chars: usize,
        iterations: usize,
    },
    Failed {
        orchestration_id: String,
        error: String,
    },
    Closed {
        orchestration_id: String,
        reason: Option<String>,
    },
}
