use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Running,
    Completed,
    Failed,
    Interrupted,
}

impl SessionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "interrupted" => Self::Interrupted,
            _ => Self::Running,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub agent_definition_id: String,
    pub agent_definition_name: String,
    pub session_key: String,
    pub parent_session_id: Option<String>,
    pub thread_id: Option<String>,
    pub source_channel: Option<String>,
    pub status: SessionStatus,
    pub model: Option<String>,
    pub turn_count: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub cost_usd: f64,
    pub transcript_path: Option<String>,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessage {
    pub id: i64,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub model: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionToolCall {
    pub id: i64,
    pub session_id: String,
    pub message_id: Option<i64>,
    pub tool_name: String,
    pub tool_input: Option<String>,
    pub tool_output: Option<String>,
    pub status: String,
    pub duration_ms: Option<i64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSearchParams {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub source_channel: Option<String>,
    #[serde(default)]
    pub parent_session_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSearchResult {
    pub sessions: Vec<SessionRecord>,
    pub total: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_status_roundtrip() {
        for status in [
            SessionStatus::Running,
            SessionStatus::Completed,
            SessionStatus::Failed,
            SessionStatus::Interrupted,
        ] {
            assert_eq!(SessionStatus::parse(status.as_str()), status);
        }
    }

    #[test]
    fn session_status_parse_unknown_defaults_to_running() {
        assert_eq!(SessionStatus::parse("bogus"), SessionStatus::Running);
        assert_eq!(SessionStatus::parse(""), SessionStatus::Running);
    }

    #[test]
    fn session_status_serde_roundtrip() {
        let status = SessionStatus::Completed;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"completed\"");
        let parsed: SessionStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, status);
    }

    #[test]
    fn session_search_params_defaults() {
        let params = SessionSearchParams::default();
        assert!(params.query.is_none());
        assert!(params.agent_id.is_none());
        assert!(params.limit.is_none());
        assert!(params.offset.is_none());
    }
}
