use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_TIMEOUT_MS: u64 = 600_000;
pub const MAX_TIMEOUT_MS: u64 = 3_600_000;
pub const MAX_OUTPUT_BYTES: usize = 1_048_576;
pub const RECENT_EVENT_LIMIT: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorStatus {
    Starting,
    Running,
    Stopped,
    TimedOut,
    Failed,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorStream {
    Stdout,
    Stderr,
}

impl MonitorStream {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorStartRequest {
    pub command: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub persistent: bool,
    #[serde(default)]
    pub category: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorStopRequest {
    pub monitor_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorReadRequest {
    pub monitor_id: String,
    #[serde(default)]
    pub max_bytes: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorEvent {
    pub monitor_id: String,
    pub thread_id: Option<String>,
    pub timestamp_ms: u64,
    pub stream: MonitorStream,
    pub line: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorSnapshot {
    pub monitor_id: String,
    pub status: MonitorStatus,
    pub description: String,
    pub command: String,
    pub output_file: PathBuf,
    pub persistent: bool,
    pub thread_id: Option<String>,
    pub session_id: Option<String>,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub output_bytes: usize,
    pub dropped_bytes: usize,
    pub recent_events: Vec<MonitorEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorStartResponse {
    pub monitor_id: String,
    pub status: MonitorStatus,
    pub description: String,
    pub output_file: PathBuf,
    pub persistent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorListResponse {
    pub monitors: Vec<MonitorSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorStopResponse {
    pub monitor_id: String,
    pub status: MonitorStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorReadResponse {
    pub monitor_id: String,
    pub output: String,
    pub truncated: bool,
    pub bytes: usize,
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
