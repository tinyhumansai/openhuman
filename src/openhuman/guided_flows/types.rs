//! Domain types for guided recommendation flows.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AnswerType {
    SingleChoice,
    MultiChoice,
    FreeText,
    Number,
    Boolean,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowStep {
    pub id: String,
    pub prompt: String,
    pub answer_type: AnswerType,
    #[serde(default)]
    pub choices: Vec<String>,
    #[serde(default)]
    pub validation: Option<String>,
    #[serde(default)]
    pub branches: HashMap<String, String>,
    pub next: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: u32,
    pub start_step: String,
    pub steps: Vec<FlowStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepAnswer {
    pub step_id: String,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlowSessionState {
    Active,
    Completed,
    Abandoned,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowSession {
    pub session_id: String,
    pub flow_id: String,
    pub state: FlowSessionState,
    pub current_step: String,
    pub answers: Vec<StepAnswer>,
    pub recommendation: Option<Recommendation>,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    pub title: String,
    pub summary: String,
    pub confidence: f64,
    pub next_actions: Vec<String>,
    pub metadata: HashMap<String, serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answer_type_serializes_snake_case() {
        let at = AnswerType::SingleChoice;
        assert_eq!(serde_json::to_string(&at).unwrap(), "\"single_choice\"");
    }

    #[test]
    fn session_state_serializes() {
        assert_eq!(
            serde_json::to_string(&FlowSessionState::Active).unwrap(),
            "\"active\""
        );
        assert_eq!(
            serde_json::to_string(&FlowSessionState::Completed).unwrap(),
            "\"completed\""
        );
    }

    #[test]
    fn recommendation_round_trips() {
        let rec = Recommendation {
            title: "Use Whisper".into(),
            summary: "Local STT".into(),
            confidence: 0.92,
            next_actions: vec!["Install whisper".into()],
            metadata: HashMap::new(),
        };
        let json = serde_json::to_string(&rec).unwrap();
        let back: Recommendation = serde_json::from_str(&json).unwrap();
        assert_eq!(back.title, "Use Whisper");
    }

    #[test]
    fn flow_definition_deserializes() {
        let json = r#"{"id":"onboarding","name":"Setup","description":"x","version":1,"start_step":"q1","steps":[{"id":"q1","prompt":"?","answer_type":"single_choice","choices":["a"],"next":null}]}"#;
        let def: FlowDefinition = serde_json::from_str(json).unwrap();
        assert_eq!(def.steps[0].answer_type, AnswerType::SingleChoice);
    }
}
