//! Public data model for high-level agent orchestration.
//!
//! The status vocabulary is TinyAgents'
//! [`OrchestrationTaskStatus`] — the host's own `AgentStatus` copy was retired
//! once the control plane moved onto the crate's `DetachedTaskRegistry`. It
//! never appeared on a JSON-RPC schema, and the detached sub-agent path was
//! already reporting the crate enum, so the two vocabularies are now one.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub use tinyagents_graph::orchestration::OrchestrationTaskStatus;

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
    pub status: OrchestrationTaskStatus,
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
    pub status: OrchestrationTaskStatus,
    pub prompt: String,
    pub result_summary: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}
