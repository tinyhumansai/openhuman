//! Google Meet integration settings.
//!
//! Currently exposes a single privacy-relevant flag:
//! `auto_orchestrator_handoff` — when `true`, ending a Google Meet call
//! inside the OpenHuman webview hands the captured transcript to the
//! orchestrator agent, which may **proactively** execute tools (e.g. post
//! summaries to Slack, draft messages, schedule follow-ups). Default
//! `false` so the user must opt in before any external action fires.
//!
//! See issue tinyhumansai/openhuman#1299.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MeetConfig {
    /// When `true`, the orchestrator agent receives the transcript of every
    /// completed Google Meet call as a fresh chat thread and is invited to
    /// take proactive actions on it (drafting messages, scheduling
    /// follow-ups, etc.). When `false` (the default), transcripts still
    /// land in memory but no auto-orchestrator handoff fires.
    #[serde(default = "default_auto_orchestrator_handoff")]
    pub auto_orchestrator_handoff: bool,
}

fn default_auto_orchestrator_handoff() -> bool {
    false
}

impl Default for MeetConfig {
    fn default() -> Self {
        Self {
            auto_orchestrator_handoff: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_disables_handoff() {
        let cfg = MeetConfig::default();
        assert!(
            !cfg.auto_orchestrator_handoff,
            "auto_orchestrator_handoff must default to false (privacy-conservative)"
        );
    }

    #[test]
    fn default_helper_returns_false() {
        assert!(!default_auto_orchestrator_handoff());
    }

    #[test]
    fn deserialize_missing_optional_fields_uses_defaults() {
        let cfg: MeetConfig = serde_json::from_value(json!({})).unwrap();
        assert!(
            !cfg.auto_orchestrator_handoff,
            "missing field must deserialize to false"
        );
    }

    #[test]
    fn deserialize_respects_explicit_handoff_flag() {
        let cfg: MeetConfig = serde_json::from_value(json!({
            "auto_orchestrator_handoff": true
        }))
        .unwrap();
        assert!(cfg.auto_orchestrator_handoff);
    }

    #[test]
    fn round_trip_preserves_handoff_flag() {
        let original = MeetConfig {
            auto_orchestrator_handoff: true,
        };
        let s = serde_json::to_string(&original).unwrap();
        let back: MeetConfig = serde_json::from_str(&s).unwrap();
        assert!(back.auto_orchestrator_handoff);
    }
}
