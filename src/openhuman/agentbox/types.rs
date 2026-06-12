use serde::{Deserialize, Serialize};
use std::time::Instant;

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
