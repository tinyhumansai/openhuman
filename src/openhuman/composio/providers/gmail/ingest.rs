//! Gmail → memory tree ingest plumbing.
//!
//! Owns the conversion from a page of `GMAIL_FETCH_EMAILS` slim-envelope
//! messages (post-processed by [`super::post_process`]) into
//! [`EmailThread`] batches grouped by `(sender, threadId)`, then drives
//! [`memory::tree::ingest::ingest_email`] per thread with
//! [`GmailMarkdownStyle::Standard`].
//!
//! Mirrors the bin's `bucket_by_sender_and_thread` grouping and the
//! per-thread rendering tested in `gmail-fetch-emails.rs`. Source-id is
//! per-inbox (`gmail:{connection_id}`), so every thread from one
//! connection lands in the same source tree.
//!
//! Idempotency: chunk IDs are content-hashed inside the memory tree, so
//! re-ingesting a previously-seen Gmail message is an UPSERT — buffer
//! token_sum may drift if content changes (rare for sealed mail), but
//! the tree's seal cascade handles that on next append.

use std::collections::BTreeMap;

use anyhow::Result;
use serde_json::Value;

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::canonicalize::email::{
    EmailMessage, EmailThread, GmailMarkdownStyle,
};
use crate::openhuman::memory::tree::canonicalize::email_clean::{
    extract_email, parse_message_date,
};
use crate::openhuman::memory::tree::ingest::{ingest_email, IngestResult};

/// Provider name embedded in the canonical email-thread header. Matches
/// the value `memory::tree::retrieval::source::PLATFORM_KINDS` expects.
pub const GMAIL_PROVIDER: &str = "gmail";

/// Tags attached to every Gmail-ingested chunk. Stable list — retrieval
/// callers filter on these.
pub const DEFAULT_TAGS: &[&str] = &["gmail", "ingested"];

/// Inner map: `threadId → ordered messages`.
type ThreadBucket<'a> = BTreeMap<String, Vec<&'a Value>>;

/// Group raw page messages by `(sender_email, thread_id)`. Within a
/// thread, messages sort ascending by date so each rendered thread reads
/// chronologically. Mirrors the same-named function in
/// `bin/gmail_fetch_emails.rs` — kept aligned so bin output and
/// production canonicalisation see the same bucketing.
pub(crate) fn bucket_by_sender_and_thread(
    msgs: &[Value],
) -> BTreeMap<String, ThreadBucket<'_>> {
    let mut out: BTreeMap<String, ThreadBucket<'_>> = BTreeMap::new();
    for m in msgs {
        let from = m.get("from").and_then(|v| v.as_str()).unwrap_or("");
        let sender = extract_email(from)
            .map(|s| s.to_lowercase())
            .unwrap_or_else(|| {
                if from.trim().is_empty() {
                    "unknown".to_string()
                } else {
                    from.trim().to_lowercase()
                }
            });
        let thread = m
            .get("threadId")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                m.get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| format!("solo:{s}"))
            })
            .unwrap_or_else(|| "unknown".to_string());
        out.entry(sender)
            .or_default()
            .entry(thread)
            .or_default()
            .push(m);
    }
    for threads in out.values_mut() {
        for msgs in threads.values_mut() {
            msgs.sort_by_key(|m| {
                parse_message_date(m).map(|d| d.timestamp()).unwrap_or(0)
            });
        }
    }
    out
}

/// Build an [`EmailMessage`] from a raw slim-envelope JSON message.
/// Returns `None` when the message has no parseable date — the rest of
/// the pipeline can't sort or canonicalise without one.
pub(crate) fn raw_to_email_message(raw: &Value) -> Option<EmailMessage> {
    let id = raw
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("");
    let from = raw
        .get("from")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let to = parse_address_list(raw.get("to"));
    let cc = parse_address_list(raw.get("cc"));
    let subject = raw
        .get("subject")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let sent_at = parse_message_date(raw)?;
    let body = raw
        .get("markdown")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let source_ref = if id.is_empty() {
        None
    } else {
        Some(format!("gmail://msg/{id}"))
    };
    Some(EmailMessage {
        from,
        to,
        cc,
        subject,
        sent_at,
        body,
        source_ref,
    })
}

/// Parse the `to` / `cc` field which Composio surfaces as either a
/// JSON array of strings or a single comma-separated string. Empty
/// entries are dropped.
fn parse_address_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|s| s.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        Some(Value::String(s)) => s
            .split(',')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// Ingest a page of raw Gmail messages into the memory tree under one
/// inbox. Each `(sender, thread)` group becomes one [`EmailThread`] with
/// [`GmailMarkdownStyle::Standard`], handed to
/// [`ingest_email`] which fans out to the chunker + scorer + source
/// tree downstream.
///
/// Returns the total number of chunks written across all threads so
/// callers can surface counts in logs / outcomes. Per-thread errors are
/// logged and swallowed — one bad thread should not abort the whole
/// page (the next sync re-fetches it via the date-cursor).
pub async fn ingest_page_into_memory_tree(
    config: &Config,
    source_id: &str,
    owner: &str,
    page_messages: &[Value],
) -> Result<usize> {
    if page_messages.is_empty() {
        return Ok(0);
    }
    let buckets = bucket_by_sender_and_thread(page_messages);
    let mut total_chunks = 0usize;
    let mut total_threads = 0usize;
    for (sender, threads) in &buckets {
        for (thread_id, raw_msgs) in threads {
            let messages: Vec<EmailMessage> =
                raw_msgs.iter().filter_map(|raw| raw_to_email_message(raw)).collect();
            if messages.is_empty() {
                log::debug!(
                    "[composio:gmail][ingest] skipping empty thread sender={sender} thread={thread_id}"
                );
                continue;
            }
            let thread_subject = pick_thread_subject(&messages);
            log::info!(
                "[composio:gmail][ingest] thread sender={} thread_id={} messages={} source_id={}",
                sender,
                thread_id,
                messages.len(),
                source_id
            );
            let thread = EmailThread {
                provider: GMAIL_PROVIDER.to_string(),
                thread_subject,
                messages,
                gmail_style: Some(GmailMarkdownStyle::Standard),
            };
            let tags = DEFAULT_TAGS.iter().map(|s| (*s).to_string()).collect();
            match ingest_email(config, source_id, owner, tags, thread).await {
                Ok(IngestResult { chunks_written, .. }) => {
                    total_chunks += chunks_written;
                    total_threads += 1;
                }
                Err(e) => {
                    log::warn!(
                        "[composio:gmail][ingest] ingest_email failed sender={} thread={} err={:#}",
                        sender,
                        thread_id,
                        e
                    );
                }
            }
        }
    }
    log::info!(
        "[composio:gmail][ingest] page_done source_id={source_id} threads={total_threads} chunks={total_chunks}"
    );
    Ok(total_chunks)
}

/// Strip "Re:" / "Fwd:" prefixes from the head message's subject so
/// every message in a thread shares one canonical thread subject. Falls
/// back to "(no subject)" when empty.
fn pick_thread_subject(messages: &[EmailMessage]) -> String {
    let raw = messages
        .first()
        .map(|m| m.subject.trim().to_string())
        .unwrap_or_default();
    let stripped = strip_reply_prefixes(&raw);
    if stripped.is_empty() {
        "(no subject)".to_string()
    } else {
        stripped
    }
}

/// Iteratively strip `Re:` / `Fwd:` / `Fw:` prefixes (case-insensitive,
/// optional whitespace) from the front of a subject. Stops once a pass
/// removes nothing.
fn strip_reply_prefixes(subject: &str) -> String {
    let mut s = subject.trim().to_string();
    loop {
        let lower = s.to_ascii_lowercase();
        let stripped = if lower.starts_with("re:") {
            Some(&s[3..])
        } else if lower.starts_with("fwd:") {
            Some(&s[4..])
        } else if lower.starts_with("fw:") {
            Some(&s[3..])
        } else {
            None
        };
        match stripped {
            Some(rest) => {
                let trimmed = rest.trim_start().to_string();
                if trimmed == s {
                    return s;
                }
                s = trimmed;
            }
            None => return s,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bucket_groups_by_sender_and_thread() {
        let msgs = vec![
            json!({
                "id": "m1", "threadId": "t1",
                "from": "Alice <alice@example.com>",
                "subject": "Hi",
                "date": "2026-04-21T10:00:00Z",
                "markdown": "first",
            }),
            json!({
                "id": "m2", "threadId": "t1",
                "from": "Alice <alice@example.com>",
                "subject": "Re: Hi",
                "date": "2026-04-21T11:00:00Z",
                "markdown": "second",
            }),
            json!({
                "id": "m3", "threadId": "t2",
                "from": "bob@example.com",
                "subject": "Other",
                "date": "2026-04-22T09:00:00Z",
                "markdown": "third",
            }),
        ];
        let buckets = bucket_by_sender_and_thread(&msgs);
        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets["alice@example.com"]["t1"].len(), 2);
        assert_eq!(buckets["bob@example.com"]["t2"].len(), 1);
        // Within a thread, messages sort ascending by date.
        let t1 = &buckets["alice@example.com"]["t1"];
        assert_eq!(t1[0].get("id").unwrap().as_str().unwrap(), "m1");
        assert_eq!(t1[1].get("id").unwrap().as_str().unwrap(), "m2");
    }

    #[test]
    fn solo_thread_id_when_threadid_missing() {
        let msgs = vec![json!({
            "id": "abc",
            "from": "noreply@github.com",
            "subject": "x",
            "date": "2026-04-21T10:00:00Z",
            "markdown": "body",
        })];
        let buckets = bucket_by_sender_and_thread(&msgs);
        assert!(buckets["noreply@github.com"].contains_key("solo:abc"));
    }

    #[test]
    fn unknown_sender_fallback_when_from_blank() {
        let msgs = vec![json!({
            "id": "abc", "threadId": "t1",
            "from": "",
            "subject": "x",
            "date": "2026-04-21T10:00:00Z",
            "markdown": "body",
        })];
        let buckets = bucket_by_sender_and_thread(&msgs);
        assert!(buckets.contains_key("unknown"));
    }

    #[test]
    fn raw_to_email_message_parses_slim_envelope() {
        let raw = json!({
            "id": "m1",
            "from": "Alice <alice@example.com>",
            "to": "me@example.com",
            "cc": "team@example.com",
            "subject": "Phoenix kickoff",
            "date": "2026-04-21T10:00:00Z",
            "markdown": "Let's ship Phoenix.",
        });
        let msg = raw_to_email_message(&raw).unwrap();
        assert_eq!(msg.from, "Alice <alice@example.com>");
        assert_eq!(msg.to, vec!["me@example.com"]);
        assert_eq!(msg.cc, vec!["team@example.com"]);
        assert_eq!(msg.subject, "Phoenix kickoff");
        assert_eq!(msg.body, "Let's ship Phoenix.");
        assert_eq!(msg.source_ref.as_deref(), Some("gmail://msg/m1"));
    }

    #[test]
    fn raw_to_email_message_handles_to_array() {
        let raw = json!({
            "id": "m1",
            "from": "a@x",
            "to": ["b@x", "c@x"],
            "subject": "x",
            "date": "2026-04-21T10:00:00Z",
            "markdown": "body",
        });
        let msg = raw_to_email_message(&raw).unwrap();
        assert_eq!(msg.to, vec!["b@x", "c@x"]);
    }

    #[test]
    fn raw_to_email_message_handles_comma_separated_to_string() {
        let raw = json!({
            "id": "m1",
            "from": "a@x",
            "to": "b@x, c@x ,d@x",
            "subject": "x",
            "date": "2026-04-21T10:00:00Z",
            "markdown": "body",
        });
        let msg = raw_to_email_message(&raw).unwrap();
        assert_eq!(msg.to, vec!["b@x", "c@x", "d@x"]);
    }

    #[test]
    fn raw_to_email_message_returns_none_on_unparseable_date() {
        let raw = json!({
            "id": "m1",
            "from": "a@x",
            "subject": "x",
            "date": "not-a-date",
            "markdown": "body",
        });
        assert!(raw_to_email_message(&raw).is_none());
    }

    #[test]
    fn raw_to_email_message_drops_source_ref_when_id_empty() {
        let raw = json!({
            "id": "",
            "from": "a@x",
            "subject": "x",
            "date": "2026-04-21T10:00:00Z",
            "markdown": "body",
        });
        let msg = raw_to_email_message(&raw).unwrap();
        assert!(msg.source_ref.is_none());
    }

    #[test]
    fn strip_reply_prefixes_removes_iterated() {
        assert_eq!(strip_reply_prefixes("Re: Re: Hi"), "Hi");
        assert_eq!(strip_reply_prefixes("Fwd: Re: Status"), "Status");
        assert_eq!(strip_reply_prefixes("RE: Question"), "Question");
        assert_eq!(strip_reply_prefixes("Fw: alert"), "alert");
        assert_eq!(strip_reply_prefixes("Plain subject"), "Plain subject");
    }

    #[test]
    fn pick_thread_subject_strips_reply_prefixes() {
        let messages = vec![EmailMessage {
            from: "a@x".into(),
            to: vec![],
            cc: vec![],
            subject: "Re: Re: Phoenix kickoff".into(),
            sent_at: chrono::Utc::now(),
            body: "body".into(),
            source_ref: None,
        }];
        assert_eq!(pick_thread_subject(&messages), "Phoenix kickoff");
    }

    #[test]
    fn pick_thread_subject_falls_back_to_no_subject() {
        let messages = vec![EmailMessage {
            from: "a@x".into(),
            to: vec![],
            cc: vec![],
            subject: "  ".into(),
            sent_at: chrono::Utc::now(),
            body: "body".into(),
            source_ref: None,
        }];
        assert_eq!(pick_thread_subject(&messages), "(no subject)");
    }
}
