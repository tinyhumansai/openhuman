//! Memory ingestion helpers for Gmail messages.
//!
//! Converts a `GmailMessage` into a `NamespaceDocumentInput` and calls
//! `MemoryClient::put_doc`. Callers (both the Tauri-side scanner and the
//! Rust-side `ops::sync_now`) converge here.

use crate::openhuman::gmail::types::GmailMessage;
use crate::openhuman::memory::store::types::NamespaceDocumentInput;
use crate::openhuman::memory::MemoryClientRef;
use serde_json::json;

/// Ingest a single `GmailMessage` into the memory layer under the namespace
/// `skill:gmail:{email}`. Returns the stored document id.
///
/// # Errors
///
/// Returns an error string if the memory `put_doc` call fails.
pub async fn ingest_message(
    memory: &MemoryClientRef,
    email: &str,
    msg: &GmailMessage,
) -> Result<String, String> {
    let namespace = format!("skill:gmail:{}", email);
    log::debug!(
        "[gmail][ingest] namespace={} key={} subject={:?} ts_ms={}",
        namespace,
        msg.id,
        msg.subject,
        msg.ts_ms
    );

    // Assemble content block that the LLM can read.
    let content = assemble_content(msg);

    // Build tag list: all labels in lower-case, plus "unread" flag.
    let mut tags: Vec<String> = msg.labels.iter().map(|l| l.to_lowercase()).collect();
    if msg.is_unread() && !tags.contains(&"unread".to_string()) {
        tags.push("unread".to_string());
    }

    let input = NamespaceDocumentInput {
        namespace,
        key: msg.id.clone(),
        title: if msg.subject.is_empty() {
            "(no subject)".to_string()
        } else {
            msg.subject.clone()
        },
        content,
        source_type: "email".to_string(),
        priority: if msg.is_unread() { "high" } else { "normal" }.to_string(),
        tags,
        metadata: json!({
            "thread_id": msg.thread_id,
            "from":      msg.from,
            "to":        msg.to,
            "ts_ms":     msg.ts_ms,
            "labels":    msg.labels,
        }),
        category: msg.primary_category().to_string(),
        session_id: None,
        document_id: None,
    };

    let doc_id = memory.put_doc(input).await?;
    log::debug!("[gmail][ingest] stored doc_id={} key={}", doc_id, msg.id);
    Ok(doc_id)
}

/// Ingest a batch of messages, returning (success_count, error_count).
pub async fn ingest_batch(
    memory: &MemoryClientRef,
    email: &str,
    messages: &[GmailMessage],
) -> (usize, usize) {
    log::info!(
        "[gmail][ingest] batch start email={} count={}",
        email,
        messages.len()
    );
    let mut ok = 0usize;
    let mut err = 0usize;
    for msg in messages {
        match ingest_message(memory, email, msg).await {
            Ok(_) => ok += 1,
            Err(e) => {
                log::warn!(
                    "[gmail][ingest] failed to ingest msg id={} subject={:?}: {}",
                    msg.id,
                    msg.subject,
                    e
                );
                err += 1;
            }
        }
    }
    log::info!(
        "[gmail][ingest] batch done email={} ok={} err={}",
        email,
        ok,
        err
    );
    (ok, err)
}

// ---------------------------------------------------------------------------
// Content assembly
// ---------------------------------------------------------------------------

/// Build a readable plain-text block for the memory layer from message fields.
fn assemble_content(msg: &GmailMessage) -> String {
    let mut parts = Vec::with_capacity(6);

    if !msg.from.is_empty() {
        parts.push(format!("From: {}", msg.from));
    }
    if !msg.to.is_empty() {
        parts.push(format!("To: {}", msg.to));
    }
    if !msg.subject.is_empty() {
        parts.push(format!("Subject: {}", msg.subject));
    }
    if msg.ts_ms > 0 {
        // Format a human-readable date from millisecond timestamp.
        use chrono::{TimeZone, Utc};
        let dt = Utc.timestamp_millis_opt(msg.ts_ms).single();
        if let Some(dt) = dt {
            parts.push(format!("Date: {}", dt.format("%Y-%m-%d %H:%M UTC")));
        }
    }
    if !parts.is_empty() {
        parts.push(String::new()); // blank separator before body
    }
    if !msg.body.is_empty() {
        parts.push(msg.body.clone());
    } else if !msg.snippet.is_empty() {
        parts.push(msg.snippet.clone());
    }
    parts.join("\n")
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_msg() -> GmailMessage {
        GmailMessage {
            id: "msg-001".into(),
            thread_id: "thread-001".into(),
            from: "alice@example.com".into(),
            to: "bob@example.com".into(),
            subject: "Hello world".into(),
            snippet: "Short preview…".into(),
            body: "Full body text here.".into(),
            labels: vec!["INBOX".into(), "UNREAD".into()],
            ts_ms: 1_700_000_000_000,
        }
    }

    #[test]
    fn assemble_content_includes_headers() {
        let msg = fixture_msg();
        let content = assemble_content(&msg);
        assert!(content.contains("From: alice@example.com"));
        assert!(content.contains("To: bob@example.com"));
        assert!(content.contains("Subject: Hello world"));
        assert!(content.contains("Full body text here."));
    }

    #[test]
    fn assemble_content_falls_back_to_snippet() {
        let mut msg = fixture_msg();
        msg.body = String::new();
        let content = assemble_content(&msg);
        assert!(content.contains("Short preview…"));
    }

    #[test]
    fn is_unread_detects_label() {
        let msg = fixture_msg();
        assert!(msg.is_unread());
    }

    #[test]
    fn primary_category_returns_inbox() {
        let msg = fixture_msg();
        assert_eq!(msg.primary_category(), "inbox");
    }

    #[test]
    fn primary_category_returns_other_for_unknown_labels() {
        let mut msg = fixture_msg();
        msg.labels = vec!["CATEGORY_PROMOTIONS".into()];
        assert_eq!(msg.primary_category(), "other");
    }
}
