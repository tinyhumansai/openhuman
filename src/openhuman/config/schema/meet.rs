//! Google Meet integration settings.
//!
//! Exposes privacy-relevant gates (`auto_orchestrator_handoff`,
//! `ingest_backend_transcripts`) and Meeting Assistant policies
//! (`auto_join_policy`, `auto_summarize_policy`, `listen_only_default`).
//!
//! See epic tinyhumansai/openhuman#3505.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Controls whether the bot auto-joins meetings from the calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AutoJoinPolicy {
    /// Prompt the user before every join (default).
    AskEachTime,
    /// Always join without prompting.
    Always,
    /// Never auto-join.
    Never,
}

impl Default for AutoJoinPolicy {
    fn default() -> Self {
        Self::AskEachTime
    }
}

/// Controls whether post-call summaries are generated automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AutoSummarizePolicy {
    /// Ask the user after the call ends (default).
    Ask,
    /// Always generate a summary.
    Always,
    /// Never generate.
    Never,
}

impl Default for AutoSummarizePolicy {
    fn default() -> Self {
        Self::Ask
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MeetConfig {
    /// When `true`, the orchestrator agent receives the transcript of every
    /// completed Google Meet call as a fresh chat thread and is invited to
    /// take proactive actions on it.
    #[serde(default = "default_auto_orchestrator_handoff")]
    pub auto_orchestrator_handoff: bool,

    /// When `true`, backend-bot meeting transcripts are ingested into the
    /// memory tree after the call ends.
    #[serde(default = "default_ingest_backend_transcripts")]
    pub ingest_backend_transcripts: bool,

    /// Whether the bot should auto-join calendar meetings with Meet links.
    #[serde(default)]
    pub auto_join_policy: AutoJoinPolicy,

    /// Whether to auto-generate a summary after a call ends.
    #[serde(default)]
    pub auto_summarize_policy: AutoSummarizePolicy,

    /// When `true`, the bot joins in listen-only mode (mic muted).
    #[serde(default = "default_listen_only")]
    pub listen_only_default: bool,
}

fn default_auto_orchestrator_handoff() -> bool {
    false
}

fn default_ingest_backend_transcripts() -> bool {
    false
}

fn default_listen_only() -> bool {
    true
}

impl Default for MeetConfig {
    fn default() -> Self {
        Self {
            auto_orchestrator_handoff: false,
            ingest_backend_transcripts: false,
            auto_join_policy: AutoJoinPolicy::default(),
            auto_summarize_policy: AutoSummarizePolicy::default(),
            listen_only_default: true,
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
        assert!(!cfg.auto_orchestrator_handoff);
    }

    #[test]
    fn default_disables_ingest_backend_transcripts() {
        let cfg = MeetConfig::default();
        assert!(!cfg.ingest_backend_transcripts);
    }

    #[test]
    fn default_auto_join_is_ask_each_time() {
        let cfg = MeetConfig::default();
        assert_eq!(cfg.auto_join_policy, AutoJoinPolicy::AskEachTime);
    }

    #[test]
    fn default_auto_summarize_is_ask() {
        let cfg = MeetConfig::default();
        assert_eq!(cfg.auto_summarize_policy, AutoSummarizePolicy::Ask);
    }

    #[test]
    fn default_listen_only_is_true() {
        let cfg = MeetConfig::default();
        assert!(cfg.listen_only_default);
    }

    #[test]
    fn deserialize_missing_fields_uses_defaults() {
        let cfg: MeetConfig = serde_json::from_value(json!({})).unwrap();
        assert!(!cfg.auto_orchestrator_handoff);
        assert!(!cfg.ingest_backend_transcripts);
        assert_eq!(cfg.auto_join_policy, AutoJoinPolicy::AskEachTime);
        assert_eq!(cfg.auto_summarize_policy, AutoSummarizePolicy::Ask);
        assert!(cfg.listen_only_default);
    }

    #[test]
    fn deserialize_explicit_policies() {
        let cfg: MeetConfig = serde_json::from_value(json!({
            "auto_join_policy": "always",
            "auto_summarize_policy": "never",
            "listen_only_default": false
        }))
        .unwrap();
        assert_eq!(cfg.auto_join_policy, AutoJoinPolicy::Always);
        assert_eq!(cfg.auto_summarize_policy, AutoSummarizePolicy::Never);
        assert!(!cfg.listen_only_default);
    }

    #[test]
    fn round_trip_preserves_all_fields() {
        let original = MeetConfig {
            auto_orchestrator_handoff: true,
            ingest_backend_transcripts: true,
            auto_join_policy: AutoJoinPolicy::Never,
            auto_summarize_policy: AutoSummarizePolicy::Always,
            listen_only_default: false,
        };
        let s = serde_json::to_string(&original).unwrap();
        let back: MeetConfig = serde_json::from_str(&s).unwrap();
        assert!(back.auto_orchestrator_handoff);
        assert!(back.ingest_backend_transcripts);
        assert_eq!(back.auto_join_policy, AutoJoinPolicy::Never);
        assert_eq!(back.auto_summarize_policy, AutoSummarizePolicy::Always);
        assert!(!back.listen_only_default);
    }
}
