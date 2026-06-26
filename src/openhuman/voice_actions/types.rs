//! Domain types for voice-driven desktop actions.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionSafety {
    Safe,
    RequiresConfirmation,
    Destructive,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IntentStatus {
    Pending,
    Confirmed,
    Executed,
    Rejected,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceIntent {
    pub id: String,
    pub utterance: String,
    pub action: String,
    pub namespace: String,
    pub function: String,
    pub confidence: f64,
    pub safety: ActionSafety,
    pub status: IntentStatus,
    pub params: serde_json::Value,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub created_at: u64,
    /// Previous intents in this conversation for multi-turn context.
    #[serde(default)]
    pub context_history: Vec<String>,
}

/// Multi-turn conversation context for voice actions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionContext {
    pub session_id: String,
    pub intents: Vec<String>,
    pub last_active: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionMapping {
    pub pattern: String,
    pub namespace: String,
    pub function: String,
    pub safety: ActionSafety,
    pub description: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safety_serializes() {
        assert_eq!(
            serde_json::to_string(&ActionSafety::Safe).unwrap(),
            "\"safe\""
        );
        assert_eq!(
            serde_json::to_string(&ActionSafety::RequiresConfirmation).unwrap(),
            "\"requires_confirmation\""
        );
    }

    #[test]
    fn intent_status_serializes() {
        assert_eq!(
            serde_json::to_string(&IntentStatus::Pending).unwrap(),
            "\"pending\""
        );
        assert_eq!(
            serde_json::to_string(&IntentStatus::Executed).unwrap(),
            "\"executed\""
        );
    }

    #[test]
    fn voice_intent_round_trips() {
        let vi = VoiceIntent {
            id: "vi-1".into(),
            utterance: "open settings".into(),
            action: "Open Settings".into(),
            namespace: "config".into(),
            function: "get".into(),
            confidence: 0.9,
            safety: ActionSafety::Safe,
            status: IntentStatus::Pending,
            params: serde_json::json!({}),
            result: None,
            error: None,
            created_at: 0,
            context_history: vec![],
        };
        let json = serde_json::to_string(&vi).unwrap();
        let back: VoiceIntent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.utterance, "open settings");
    }
}
