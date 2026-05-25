//! Input shapes for archivist.
//!
//! Two distinct types because they cover two distinct flows:
//!
//! - [`Turn`]         — input to the batch `archive_to_tree` flow
//!                      (clip-and-push-to-tree).
//! - [`ArchivedTurn`] — per-turn capture record persisted as a single md
//!                      file under `<content_root>/episodic/<session>/<seq>.md`.
//!                      Mirrors the legacy `unified::fts5::EpisodicEntry` so
//!                      the harness archivist can dual-write while we
//!                      migrate off FTS5.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One per-turn capture record persisted by [`crate::openhuman::memory_archivist::store::record_turn`].
/// Field names match the legacy `EpisodicEntry` so the harness archivist
/// can call into both surfaces with the same payload during the
/// migration window.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ArchivedTurn {
    pub session_id: String,
    /// Per-session sequence number, assigned by `record_turn` on write.
    pub seq: u32,
    /// Wall-clock timestamp of the turn (epoch milliseconds).
    pub timestamp_ms: i64,
    /// `"user"` / `"assistant"` / `"system"` / `"tool"`.
    pub role: String,
    pub content: String,
    /// Optional post-turn lesson (kept verbatim from the harness).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lesson: Option<String>,
    /// Serialized tool-call payload, when the turn issued any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls_json: Option<String>,
    /// Cost in microdollars; 0 when not yet billed.
    #[serde(default)]
    pub cost_microdollars: u64,
}

/// One conversation turn. `tool_calls_json` carries the raw model-side
/// tool-call payload when present; [`crate::openhuman::memory_archivist::clean_conversation`]
/// strips it before the turn lands in the tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    /// `"user"` / `"assistant"` / `"system"` / `"tool"` — free-form so we
    /// don't fight any specific harness's role taxonomy.
    pub role: String,
    /// Natural-language body.
    pub content: String,
    /// Raw JSON of any tool invocations the turn issued. Dropped during
    /// clipping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls_json: Option<String>,
    /// Wall-clock timestamp the turn occurred. Used as the tree leaf
    /// timestamp.
    pub timestamp: DateTime<Utc>,
}

impl Turn {
    pub fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            tool_calls_json: None,
            timestamp: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn archived_turn_defaults_are_empty_and_zero() {
        let turn = ArchivedTurn::default();
        assert!(turn.session_id.is_empty());
        assert_eq!(turn.seq, 0);
        assert_eq!(turn.timestamp_ms, 0);
        assert!(turn.role.is_empty());
        assert!(turn.content.is_empty());
        assert!(turn.lesson.is_none());
        assert!(turn.tool_calls_json.is_none());
        assert_eq!(turn.cost_microdollars, 0);
    }

    #[test]
    fn turn_new_sets_role_content_and_no_tool_calls() {
        let turn = Turn::new("user", "hello");
        assert_eq!(turn.role, "user");
        assert_eq!(turn.content, "hello");
        assert!(turn.tool_calls_json.is_none());
    }

    #[test]
    fn archived_turn_serde_skips_absent_optional_fields() {
        let turn = ArchivedTurn {
            session_id: "s1".into(),
            seq: 1,
            timestamp_ms: 123,
            role: "assistant".into(),
            content: "done".into(),
            lesson: None,
            tool_calls_json: None,
            cost_microdollars: 55,
        };
        let value = serde_json::to_value(&turn).unwrap();
        assert_eq!(value["session_id"], json!("s1"));
        assert!(value.get("lesson").is_none());
        assert!(value.get("tool_calls_json").is_none());

        let decoded: ArchivedTurn = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, turn);
    }
}
