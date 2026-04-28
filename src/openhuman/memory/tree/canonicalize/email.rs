//! Email threads → canonical Markdown.
//!
//! Email sources are scoped by **thread**. One thread becomes one
//! [`CanonicalisedSource`]. Headers (From, To, Subject, Date) surface in a
//! small frontmatter-style block per message; the body follows as markdown.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{normalize_source_ref, CanonicalisedSource};
use crate::openhuman::memory::tree::types::{Metadata, SourceKind};

/// One email in a thread.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmailMessage {
    pub from: String,
    #[serde(default)]
    pub to: Vec<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    pub subject: String,
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub sent_at: DateTime<Utc>,
    /// Plain-text or markdown body.
    pub body: String,
    /// Message-id header or provider URL; used for citation back to source.
    #[serde(default)]
    pub source_ref: Option<String>,
}

/// A whole email thread.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmailThread {
    /// Provider name used in the header (e.g. `gmail`, `outlook`).
    pub provider: String,
    /// Thread subject shown on top (usually the subject of the first message).
    pub thread_subject: String,
    /// Ordered messages (chronological; adapter sorts defensively).
    pub messages: Vec<EmailMessage>,
}

pub fn canonicalise(
    source_id: &str,
    owner: &str,
    tags: &[String],
    thread: EmailThread,
) -> Result<Option<CanonicalisedSource>, String> {
    if thread.messages.is_empty() {
        return Ok(None);
    }
    let mut messages = thread.messages;
    messages.sort_by_key(|m| m.sent_at);

    let first_ts = messages.first().map(|m| m.sent_at).unwrap();
    let last_ts = messages.last().map(|m| m.sent_at).unwrap();

    let mut md = String::new();
    // No leading `# Email thread — ...` header. Provider / subject info
    // belongs in the MD front-matter (Phase MD-content). The chunker splits
    // this output at `---\nFrom:` boundaries so each message becomes one chunk.

    for msg in &messages {
        md.push_str("---\n");
        md.push_str(&format!("From: {}\n", msg.from));
        if !msg.to.is_empty() {
            md.push_str(&format!("To: {}\n", msg.to.join(", ")));
        }
        if !msg.cc.is_empty() {
            md.push_str(&format!("Cc: {}\n", msg.cc.join(", ")));
        }
        md.push_str(&format!("Subject: {}\n", msg.subject));
        md.push_str(&format!("Date: {}\n\n", msg.sent_at.to_rfc3339()));
        md.push_str(msg.body.trim());
        md.push_str("\n\n");
    }

    let source_ref = normalize_source_ref(messages.first().and_then(|m| m.source_ref.clone()));

    Ok(Some(CanonicalisedSource {
        markdown: md,
        metadata: Metadata {
            source_kind: SourceKind::Email,
            source_id: source_id.to_string(),
            owner: owner.to_string(),
            timestamp: first_ts,
            time_range: (first_ts, last_ts),
            tags: tags.to_vec(),
            source_ref,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn email(ts_ms: i64, from: &str, subject: &str, body: &str) -> EmailMessage {
        EmailMessage {
            from: from.to_string(),
            to: vec!["alice@example.com".into()],
            cc: vec![],
            subject: subject.to_string(),
            sent_at: Utc.timestamp_millis_opt(ts_ms).unwrap(),
            body: body.to_string(),
            source_ref: Some(format!("<msg-{ts_ms}@example.com>")),
        }
    }

    #[test]
    fn empty_thread_returns_none() {
        let t = EmailThread {
            provider: "gmail".into(),
            thread_subject: "x".into(),
            messages: vec![],
        };
        assert!(canonicalise("gmail:t1", "alice", &[], t).unwrap().is_none());
    }

    #[test]
    fn renders_headers_and_body_per_message() {
        let t = EmailThread {
            provider: "gmail".into(),
            thread_subject: "Launch".into(),
            messages: vec![
                email(1000, "bob@example.com", "Launch", "let's ship"),
                email(2000, "alice@example.com", "Re: Launch", "agreed"),
            ],
        };
        let out = canonicalise("gmail:t1", "alice@example.com", &[], t)
            .unwrap()
            .unwrap();
        // No leading `# Email thread` header — that info belongs in front-matter.
        assert!(
            !out.markdown.contains("# Email thread — gmail — Launch"),
            "canonical email MD must NOT contain a `# ` header"
        );
        assert!(out.markdown.contains("From: bob@example.com"));
        assert!(out.markdown.contains("Subject: Launch"));
        assert!(out.markdown.contains("let's ship"));
        assert!(out.markdown.contains("Re: Launch"));
        assert!(out.markdown.contains("agreed"));
    }

    #[test]
    fn time_range_spans_thread() {
        let t = EmailThread {
            provider: "gmail".into(),
            thread_subject: "x".into(),
            messages: vec![
                email(3000, "c", "y", "third"),
                email(1000, "a", "y", "first"),
                email(2000, "b", "y", "second"),
            ],
        };
        let out = canonicalise("gmail:t1", "a", &[], t).unwrap().unwrap();
        assert_eq!(out.metadata.time_range.0.timestamp_millis(), 1000);
        assert_eq!(out.metadata.time_range.1.timestamp_millis(), 3000);
    }

    #[test]
    fn source_ref_from_first_message() {
        let t = EmailThread {
            provider: "gmail".into(),
            thread_subject: "x".into(),
            messages: vec![email(1000, "a", "y", "b"), email(2000, "b", "y", "c")],
        };
        let out = canonicalise("gmail:t1", "a", &[], t).unwrap().unwrap();
        assert_eq!(
            out.metadata.source_ref.as_ref().unwrap().value,
            "<msg-1000@example.com>"
        );
    }

    #[test]
    fn blank_source_ref_is_dropped() {
        let mut first = email(1000, "a", "y", "b");
        first.source_ref = Some("".into());
        let t = EmailThread {
            provider: "gmail".into(),
            thread_subject: "x".into(),
            messages: vec![first],
        };
        let out = canonicalise("gmail:t1", "a", &[], t).unwrap().unwrap();
        assert!(out.metadata.source_ref.is_none());
    }
}
