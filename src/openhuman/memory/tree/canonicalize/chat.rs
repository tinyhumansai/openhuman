//! Chat transcripts → canonical Markdown.
//!
//! Chat sources are scoped by **channel or group**. A batch of chat messages
//! from the same channel becomes one [`CanonicalisedSource`]; the chunker
//! slices it by token budget downstream.
//!
//! Output format (no leading `# ...` header — that info lives in front-matter
//! once Phase MD-content lands; the chunker splits at `## ` boundaries):
//! ```md
//! ## 2026-04-21T10:12:00Z — Alice
//! Message body here.
//!
//! ## 2026-04-21T10:12:40Z — Bob
//! Reply body here.
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{normalize_source_ref, CanonicalisedSource};
use crate::openhuman::memory::tree::types::{Metadata, SourceKind};

/// One chat message in a channel/group.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Author display name or id.
    pub author: String,
    /// When the message was sent.
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub timestamp: DateTime<Utc>,
    /// Plain text / markdown body.
    pub text: String,
    /// Optional per-message provenance pointer (permalink or `platform://...`).
    #[serde(default)]
    pub source_ref: Option<String>,
}

/// Adapter input — a batch of messages from one logical channel.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatBatch {
    /// Platform name used in the header (e.g. `slack`, `discord`, `telegram`).
    pub platform: String,
    /// Human-readable channel / group name for the header.
    pub channel_label: String,
    /// Ordered messages (chronological; adapter sorts defensively).
    pub messages: Vec<ChatMessage>,
}

/// Canonicalise a chat batch.
///
/// Returns `Ok(None)` if the batch has zero messages — callers treat that as
/// "nothing to ingest" and skip.
pub fn canonicalise(
    source_id: &str,
    owner: &str,
    tags: &[String],
    batch: ChatBatch,
) -> Result<Option<CanonicalisedSource>, String> {
    if batch.messages.is_empty() {
        return Ok(None);
    }
    let mut messages = batch.messages;
    messages.sort_by_key(|m| m.timestamp);

    let first_ts = messages.first().map(|m| m.timestamp).unwrap();
    let last_ts = messages.last().map(|m| m.timestamp).unwrap();

    let mut md = String::new();
    // No leading `# Chat transcript — ...` header. Platform / channel info
    // belongs in the MD front-matter (Phase MD-content). The chunker splits
    // this output at `## ` boundaries so each message becomes one chunk.
    for msg in &messages {
        md.push_str(&format!(
            "## {} — {}\n{}\n\n",
            msg.timestamp.to_rfc3339(),
            msg.author,
            msg.text.trim()
        ));
    }

    // Provenance points at the batch's first message by default (or whatever
    // the caller passed on the first message).
    let source_ref = normalize_source_ref(messages.first().and_then(|m| m.source_ref.clone()));

    let metadata = Metadata {
        source_kind: SourceKind::Chat,
        source_id: source_id.to_string(),
        owner: owner.to_string(),
        timestamp: first_ts,
        time_range: (first_ts, last_ts),
        tags: tags.to_vec(),
        source_ref,
    };
    Ok(Some(CanonicalisedSource {
        markdown: md,
        metadata,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn msg(ts_ms: i64, author: &str, text: &str) -> ChatMessage {
        ChatMessage {
            author: author.to_string(),
            timestamp: Utc.timestamp_millis_opt(ts_ms).unwrap(),
            text: text.to_string(),
            source_ref: Some(format!("slack://x/{ts_ms}")),
        }
    }

    #[test]
    fn empty_batch_returns_none() {
        let b = ChatBatch {
            platform: "slack".into(),
            channel_label: "#eng".into(),
            messages: vec![],
        };
        assert!(canonicalise("slack:#eng", "alice", &[], b)
            .unwrap()
            .is_none());
    }

    #[test]
    fn messages_are_sorted_and_range_captured() {
        let b = ChatBatch {
            platform: "slack".into(),
            channel_label: "#eng".into(),
            messages: vec![
                msg(2000, "bob", "second"),
                msg(1000, "alice", "first"),
                msg(3000, "carol", "third"),
            ],
        };
        let out = canonicalise("slack:#eng", "alice", &["eng".into()], b)
            .unwrap()
            .unwrap();
        assert_eq!(out.metadata.time_range.0.timestamp_millis(), 1000);
        assert_eq!(out.metadata.time_range.1.timestamp_millis(), 3000);
        // Check order in markdown
        let pos_first = out.markdown.find("first").unwrap();
        let pos_second = out.markdown.find("second").unwrap();
        let pos_third = out.markdown.find("third").unwrap();
        assert!(pos_first < pos_second);
        assert!(pos_second < pos_third);
    }

    #[test]
    fn includes_per_message_sections_without_header() {
        let b = ChatBatch {
            platform: "slack".into(),
            channel_label: "#eng".into(),
            messages: vec![msg(1000, "alice", "hello")],
        };
        let out = canonicalise("slack:#eng", "alice", &[], b)
            .unwrap()
            .unwrap();
        // No leading `# Chat transcript` header — that info belongs in front-matter.
        assert!(
            !out.markdown.starts_with("# "),
            "canonical chat MD must NOT start with a `# ` header"
        );
        assert!(
            out.markdown.starts_with("## "),
            "must start with first `## ` message block"
        );
        assert!(out.markdown.contains("— alice"));
        assert!(out.markdown.contains("hello"));
    }

    #[test]
    fn source_ref_taken_from_first_message() {
        let b = ChatBatch {
            platform: "slack".into(),
            channel_label: "#eng".into(),
            messages: vec![msg(1000, "alice", "hi"), msg(2000, "bob", "hey")],
        };
        let out = canonicalise("slack:#eng", "alice", &[], b)
            .unwrap()
            .unwrap();
        assert_eq!(
            out.metadata.source_ref.as_ref().unwrap().value,
            "slack://x/1000"
        );
    }

    #[test]
    fn metadata_carries_owner_and_tags() {
        let b = ChatBatch {
            platform: "slack".into(),
            channel_label: "#eng".into(),
            messages: vec![msg(1000, "alice", "hi")],
        };
        let out = canonicalise(
            "slack:#eng",
            "alice@example.com",
            &["eng".into(), "on-call".into()],
            b,
        )
        .unwrap()
        .unwrap();
        assert_eq!(out.metadata.owner, "alice@example.com");
        assert_eq!(out.metadata.tags, vec!["eng", "on-call"]);
        assert_eq!(out.metadata.source_kind, SourceKind::Chat);
    }

    #[test]
    fn blank_source_ref_is_dropped() {
        let mut first = msg(1000, "alice", "hi");
        first.source_ref = Some("   ".into());
        let b = ChatBatch {
            platform: "slack".into(),
            channel_label: "#eng".into(),
            messages: vec![first],
        };
        let out = canonicalise("slack:#eng", "alice", &[], b)
            .unwrap()
            .unwrap();
        assert!(out.metadata.source_ref.is_none());
    }
}
