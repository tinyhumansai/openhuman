//! LLM-based intent extraction for voice actions.
//!
//! Uses the existing `create_chat_provider` infrastructure to extract
//! structured intents from complex natural language utterances. Falls back
//! to pattern matching when no LLM provider is available.
//!
//! ## Architecture
//!
//! Two-tier intent resolution:
//! 1. **Fast path**: Pattern matching for known commands (0ms, no LLM)
//! 2. **LLM path**: Structured JSON extraction for complex/ambiguous utterances
//!
//! ## Log prefix
//!
//! `[voice-actions-llm]`

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use super::types::ActionSafety;

/// Structured intent extracted by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedIntent {
    /// The action to perform (e.g., "open_settings", "search", "send_message").
    pub action: String,
    /// Confidence score from the LLM (0.0–1.0).
    pub confidence: f64,
    /// Extracted parameters/slots.
    pub params: serde_json::Value,
    /// Safety classification.
    pub safety: ActionSafety,
    /// Human-readable description of what will happen.
    pub description: String,
}

/// Build the system prompt for intent extraction.
///
/// The prompt instructs the LLM to output structured JSON with the intent,
/// parameters, confidence, and safety level.
pub fn build_intent_prompt(available_actions: &[(String, String, ActionSafety)]) -> String {
    let mut actions_list = String::new();
    for (namespace, function, safety) in available_actions {
        let safety_str = match safety {
            ActionSafety::Safe => "safe",
            ActionSafety::RequiresConfirmation => "requires_confirmation",
            ActionSafety::Destructive => "destructive",
        };
        actions_list.push_str(&format!(
            "- {namespace}.{function} (safety: {safety_str})\n"
        ));
    }

    format!(
        r#"You are an intent extraction engine for a desktop voice assistant.
Given a user utterance, extract the intent as structured JSON.

Available actions:
{actions_list}
Respond with ONLY valid JSON in this exact format:
{{
  "action": "<namespace>.<function>",
  "confidence": <0.0-1.0>,
  "params": {{}},
  "safety": "safe|requires_confirmation|destructive",
  "description": "<what this action will do>"
}}

If the utterance doesn't match any available action, respond:
{{
  "action": "unknown",
  "confidence": 0.0,
  "params": {{}},
  "safety": "safe",
  "description": "No matching action found"
}}

Rules:
- Extract parameters from the utterance (e.g., "search for cats" → params: {{"query": "cats"}})
- Set confidence based on how clearly the utterance maps to an action
- Classify safety correctly: anything that sends, deletes, or modifies requires confirmation
- Be conservative: if unsure, set confidence < 0.5"#
    )
}

/// Build the user message for a specific utterance.
pub fn build_user_message(utterance: &str) -> String {
    format!("User said: \"{utterance}\"")
}

/// Parse the LLM's JSON response into an ExtractedIntent.
///
/// Handles malformed responses gracefully — returns None if parsing fails.
pub fn parse_llm_response(response: &str) -> Option<ExtractedIntent> {
    // Try to find JSON in the response (LLM might add markdown fences).
    let json_str = extract_json_from_response(response)?;

    let parsed: serde_json::Value = serde_json::from_str(&json_str).ok()?;

    let action = parsed.get("action")?.as_str()?.to_string();
    let confidence = parsed.get("confidence")?.as_f64().unwrap_or(0.0);
    let params = parsed
        .get("params")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    let safety_str = parsed.get("safety")?.as_str().unwrap_or("safe");
    let description = parsed
        .get("description")
        .and_then(|d| d.as_str())
        .unwrap_or("Unknown action")
        .to_string();

    let safety = match safety_str {
        "requires_confirmation" => ActionSafety::RequiresConfirmation,
        "destructive" => ActionSafety::Destructive,
        _ => ActionSafety::Safe,
    };

    if action == "unknown" {
        debug!("[voice-actions-llm] LLM returned unknown action");
        return None;
    }

    Some(ExtractedIntent {
        action,
        confidence,
        params,
        safety,
        description,
    })
}

/// Extract JSON from an LLM response that might contain markdown fences.
fn extract_json_from_response(response: &str) -> Option<String> {
    let trimmed = response.trim();

    // Direct JSON.
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        return Some(trimmed.to_string());
    }

    // Markdown code fence: ```json ... ``` or ``` ... ```
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            if end > start {
                return Some(trimmed[start..=end].to_string());
            }
        }
    }

    warn!("[voice-actions-llm] could not extract JSON from LLM response");
    None
}

/// Determine if an utterance should use the LLM path or pattern matching.
///
/// Returns true if the utterance is complex enough to warrant an LLM call.
/// Simple, direct commands (e.g., "open settings") use pattern matching.
pub fn should_use_llm(utterance: &str, pattern_confidence: Option<f64>) -> bool {
    // If pattern matching found a high-confidence match, skip LLM.
    if let Some(conf) = pattern_confidence {
        if conf > 0.7 {
            return false;
        }
    }

    let word_count = utterance.split_whitespace().count();

    // Very short utterances (1-2 words) are likely direct commands.
    if word_count <= 2 {
        return false;
    }

    // Long or complex utterances benefit from LLM understanding.
    if word_count >= 5 {
        return true;
    }

    // Utterances with conjunctions, conditionals, or ambiguity.
    let complex_markers = [
        "and then",
        "after that",
        "if",
        "when",
        "please",
        "could you",
        "can you",
        "I want to",
        "I need to",
    ];
    for marker in &complex_markers {
        if utterance.to_lowercase().contains(marker) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_prompt_includes_actions() {
        let actions = vec![
            ("config".into(), "get".into(), ActionSafety::Safe),
            (
                "channels".into(),
                "send".into(),
                ActionSafety::RequiresConfirmation,
            ),
        ];
        let prompt = build_intent_prompt(&actions);
        assert!(prompt.contains("config.get"));
        assert!(prompt.contains("channels.send"));
        assert!(prompt.contains("requires_confirmation"));
    }

    #[test]
    fn parse_valid_json_response() {
        let response = r#"{"action": "memory.search", "confidence": 0.9, "params": {"query": "meeting notes"}, "safety": "safe", "description": "Search for meeting notes"}"#;
        let intent = parse_llm_response(response).unwrap();
        assert_eq!(intent.action, "memory.search");
        assert_eq!(intent.confidence, 0.9);
        assert_eq!(intent.params["query"], "meeting notes");
        assert_eq!(intent.safety, ActionSafety::Safe);
    }

    #[test]
    fn parse_markdown_fenced_response() {
        let response = "```json\n{\"action\": \"config.get\", \"confidence\": 0.8, \"params\": {}, \"safety\": \"safe\", \"description\": \"Open settings\"}\n```";
        let intent = parse_llm_response(response).unwrap();
        assert_eq!(intent.action, "config.get");
    }

    #[test]
    fn parse_unknown_action_returns_none() {
        let response = r#"{"action": "unknown", "confidence": 0.0, "params": {}, "safety": "safe", "description": "No match"}"#;
        assert!(parse_llm_response(response).is_none());
    }

    #[test]
    fn parse_malformed_json_returns_none() {
        assert!(parse_llm_response("not json at all").is_none());
        assert!(parse_llm_response("").is_none());
        assert!(parse_llm_response("{incomplete").is_none());
    }

    #[test]
    fn parse_destructive_safety() {
        let response = r#"{"action": "memory.delete", "confidence": 0.95, "params": {"target": "all"}, "safety": "destructive", "description": "Delete all data"}"#;
        let intent = parse_llm_response(response).unwrap();
        assert_eq!(intent.safety, ActionSafety::Destructive);
    }

    #[test]
    fn should_use_llm_short_utterance() {
        assert!(!should_use_llm("open settings", None));
        assert!(!should_use_llm("search", None));
    }

    #[test]
    fn should_use_llm_complex_utterance() {
        assert!(should_use_llm(
            "can you search for the meeting notes from last Tuesday",
            None
        ));
        assert!(should_use_llm(
            "I want to send a message to Alice about the project",
            None
        ));
    }

    #[test]
    fn should_use_llm_high_pattern_confidence_skips() {
        // Even complex utterance skips LLM if pattern matching is confident.
        assert!(!should_use_llm(
            "can you open settings for me please",
            Some(0.85)
        ));
    }

    #[test]
    fn should_use_llm_low_pattern_confidence_uses_llm() {
        assert!(should_use_llm(
            "I need to find something about the project deadline",
            Some(0.3)
        ));
    }

    #[test]
    fn extract_json_direct() {
        let json = r#"{"key": "value"}"#;
        assert_eq!(extract_json_from_response(json).unwrap(), json);
    }

    #[test]
    fn extract_json_with_surrounding_text() {
        let response = "Here's the result:\n{\"action\": \"test\"}\nDone.";
        let extracted = extract_json_from_response(response).unwrap();
        assert_eq!(extracted, "{\"action\": \"test\"}");
    }

    #[test]
    fn build_user_message_formats_correctly() {
        let msg = build_user_message("open the settings panel");
        assert!(msg.contains("open the settings panel"));
        assert!(msg.starts_with("User said:"));
    }
}
