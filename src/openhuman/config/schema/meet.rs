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

    /// When `true`, backend-bot (Recall.ai) meeting transcripts are ingested
    /// into the memory tree after the call ends. Defaults to `false` so users
    /// must explicitly opt in before meeting content is written to durable
    /// memory — privacy-conservative default.
    #[serde(default = "default_ingest_backend_transcripts")]
    pub ingest_backend_transcripts: bool,
}

fn default_auto_orchestrator_handoff() -> bool {
    false
}

fn default_ingest_backend_transcripts() -> bool {
    false
}

impl Default for MeetConfig {
    fn default() -> Self {
        Self {
            auto_orchestrator_handoff: false,
            ingest_backend_transcripts: false,
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
    fn default_disables_ingest_backend_transcripts() {
        let cfg = MeetConfig::default();
        assert!(
            !cfg.ingest_backend_transcripts,
            "ingest_backend_transcripts must default to false (opt-in)"
        );
    }

    #[test]
    fn default_helper_returns_false() {
        assert!(!default_auto_orchestrator_handoff());
        assert!(!default_ingest_backend_transcripts());
    }

    #[test]
    fn deserialize_missing_optional_fields_uses_defaults() {
        let cfg: MeetConfig = serde_json::from_value(json!({})).unwrap();
        assert!(
            !cfg.auto_orchestrator_handoff,
            "missing field must deserialize to false"
        );
        assert!(
            !cfg.ingest_backend_transcripts,
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
    fn deserialize_respects_ingest_backend_transcripts_flag() {
        let cfg: MeetConfig = serde_json::from_value(json!({
            "ingest_backend_transcripts": true
        }))
        .unwrap();
        assert!(cfg.ingest_backend_transcripts);
    }

    #[test]
    fn round_trip_preserves_handoff_flag() {
        let original = MeetConfig {
            auto_orchestrator_handoff: true,
            ingest_backend_transcripts: true,
        };
        let s = serde_json::to_string(&original).unwrap();
        let back: MeetConfig = serde_json::from_str(&s).unwrap();
        assert!(back.auto_orchestrator_handoff);
        assert!(back.ingest_backend_transcripts);
    }
}
