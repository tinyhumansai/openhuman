use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Read-only AgentBox runtime status, surfaced over JSON-RPC for the desktop
/// control panel. Never includes the GMI API key.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AgentBoxStatus {
    /// Whether `OPENHUMAN_AGENTBOX_MODE=1` — i.e. `/run` + `/jobs` are mounted
    /// and inference is routed to GMI MaaS.
    pub mode_enabled: bool,
    /// Whether the `GMI_MAAS_*` env vars are all present and non-blank, so the
    /// provider could be registered.
    pub provider_configured: bool,
    /// Provider wiring (no secret). `None` when not configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<AgentBoxProviderInfo>,
}

/// Non-secret view of the GMI MaaS provider wiring.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AgentBoxProviderInfo {
    /// Stable provider slug used in `cloud_providers` / auth-profiles.
    pub slug: String,
    /// OpenAI-compatible base URL the agent calls.
    pub base_url: String,
    /// Model id all agent workloads are routed to.
    pub model: String,
}

/// Wire-format request: `POST /run` body.
#[derive(Debug, Clone, Deserialize)]
pub struct RunRequest {
    pub payload: RunPayload,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RunPayload {
    pub message: String,
    #[serde(default)]
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunResponse {
    pub job_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RunResult {
    pub message: String,
    pub thread_id: String,
}

/// Wire-format response: `GET /jobs/{job_id}` body.
#[derive(Debug, Clone, Serialize)]
pub struct JobView {
    pub status: JobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<RunResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Internal store record.
#[derive(Debug, Clone)]
pub struct JobRecord {
    pub status: JobStatus,
    pub result: Option<RunResult>,
    pub error: Option<String>,
    pub created_at: Instant,
    pub terminal_at: Option<Instant>,
}

impl JobRecord {
    pub fn new_pending() -> Self {
        Self {
            status: JobStatus::Pending,
            result: None,
            error: None,
            created_at: Instant::now(),
            terminal_at: None,
        }
    }

    pub fn view(&self) -> JobView {
        JobView {
            status: self.status,
            result: self.result.clone(),
            error: self.error.clone(),
        }
    }
}
