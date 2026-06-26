//! Domain types for operator inbox assistant.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageSource {
    Email,
    Chat,
    Social,
    Webhook,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TriagePriority {
    Urgent,
    High,
    Normal,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TriageStatus {
    Pending,
    Drafted,
    Sent,
    Archived,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReplyTone {
    Professional,
    Casual,
    Formal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriageRecord {
    pub id: String,
    pub source: MessageSource,
    pub sender: String,
    pub subject: String,
    pub body_preview: String,
    pub priority: TriagePriority,
    pub reason: String,
    pub proposed_reply: Option<String>,
    pub follow_up_at: Option<u64>,
    pub status: TriageStatus,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftReply {
    pub id: String,
    pub triage_id: String,
    pub content: String,
    pub tone: ReplyTone,
    pub created_at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_serializes() {
        assert_eq!(
            serde_json::to_string(&MessageSource::Email).unwrap(),
            "\"email\""
        );
    }
    #[test]
    fn priority_order() {
        assert!(TriagePriority::Urgent < TriagePriority::Low);
    }
    #[test]
    fn status_serializes() {
        assert_eq!(
            serde_json::to_string(&TriageStatus::Drafted).unwrap(),
            "\"drafted\""
        );
    }
}
