//! Gmail → memory tree ingest plumbing.
//!
//! Owns the conversion from a page of `GMAIL_FETCH_EMAILS` slim-envelope
//! messages (post-processed by [`super::post_process`]) into
//! [`EmailThread`] batches grouped by the sorted set of distinct
//! participants (`from` ∪ `to`-list, CC ignored), then drives
//! [`memory::tree::ingest::ingest_email`] per participant group.
//!
//! Source-id is `gmail:{participants}` where participants is
//! `addr1|addr2|...` (sorted, deduped, lowercased bare emails). All
//! correspondence between the same set of people lands in one source tree.
//!
//! Idempotency: chunk IDs are content-hashed inside the memory tree, so
//! re-ingesting a previously-seen Gmail message is an UPSERT — buffer
//! token_sum may drift if content changes (rare for sealed mail), but
//! the tree's seal cascade handles that on next append.

use std::collections::BTreeMap;

use anyhow::Result;
use serde_json::Value;

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::canonicalize::email::{EmailMessage, EmailThread};
use crate::openhuman::memory::tree::canonicalize::email_clean::{
    extract_email, parse_message_date,
};
use crate::openhuman::memory::tree::ingest::{ingest_email, IngestResult};
use crate::openhuman::memory::tree::util::redact::redact;

/// Provider name embedded in the canonical email-thread header. Matches
/// the value `memory::tree::retrieval::source::PLATFORM_KINDS` expects.
pub const GMAIL_PROVIDER: &str = "gmail";

/// Tags attached to every Gmail-ingested chunk. Stable list — retrieval
/// callers filter on these.
pub const DEFAULT_TAGS: &[&str] = &["gmail", "ingested"];

/// Group raw page messages by the sorted set of distinct participants
/// (`from` ∪ `to`-list). CC is deliberately excluded from the bucket key
/// so CC-only recipients don't fragment conversations. All messages
/// between the same set of people land in the same bucket regardless of
/// direction or thread ID.
///
/// The bucket key is the participants joined with `|` in sorted order,
/// e.g. `"alice@x.com|bob@y.com"`. Messages within a bucket are sorted
/// ascending by date so the rendered conversation reads chronologically.
pub(crate) fn bucket_by_participants(msgs: &[Value]) -> BTreeMap<String, Vec<&Value>> {
    let mut out: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    for m in msgs {
        let bucket_key = participants_bucket_key(m);
        if bucket_key == "__skip__" {
            // Message has no parseable addresses AND no id — drop it and warn.
            // Nothing useful can be done with it: no participants means no
            // source tree, and no id means no unique bucket either.
            log::warn!(
                "[composio:gmail][bucket] dropping message with no parseable addresses and no id"
            );
            continue;
        }
        out.entry(bucket_key).or_default().push(m);
    }
    for bucket in out.values_mut() {
        bucket.sort_by_key(|m| {
            parse_message_date(m)
                .map(|d: chrono::DateTime<chrono::Utc>| d.timestamp())
                .unwrap_or(0)
        });
    }
    out
}

/// Compute the participants bucket key for a single raw message.
///
/// Collects `from` ∪ `to` (as bare lowercased email addresses), sorts
/// and dedupes them, then joins with `|`.
///
/// **Fallback policy when all addresses fail to parse**:
/// - If the message has a non-empty `id`, use `"orphan:{id}"` so each
///   malformed message gets its own bucket and its own source tree. Two
///   messages with different ids that both fail address parsing will NOT
///   collapse into a single `"unknown"` bucket.
/// - If even `id` is missing or empty, the caller (`bucket_by_participants`)
///   should skip the message (log a warn and drop it). This function signals
///   that case by returning the sentinel `"__skip__"`.
fn participants_bucket_key(raw: &Value) -> String {
    let from = extract_email(raw.get("from").and_then(|v| v.as_str()).unwrap_or(""))
        .map(|s| s.to_lowercase())
        .filter(|s| !s.is_empty());

    let to_emails: Vec<String> = parse_address_list_for_bucket(raw.get("to"))
        .into_iter()
        .filter_map(|addr| extract_email(&addr).map(|s| s.to_lowercase()))
        .collect();

    let mut all: Vec<String> = from.into_iter().chain(to_emails).collect();
    all.sort();
    all.dedup();
    all.retain(|s| !s.is_empty());

    if all.is_empty() {
        // No parseable addresses — fall back to per-message uniqueness to
        // avoid collapsing all malformed messages into one "unknown" source
        // tree. Each orphan message gets its own bucket so nothing is silently
        // lost in a mixed pile.
        let id = raw
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());
        match id {
            Some(msg_id) => format!("orphan:{}", msg_id),
            None => {
                // id is missing: signal caller to skip this message entirely.
                "__skip__".to_string()
            }
        }
    } else {
        all.join("|")
    }
}

/// Parse the `to` / `cc` field for bucket-key construction. Handles both
/// JSON array and comma-separated string forms. Returns raw address
/// strings (may include display names); callers must extract the bare
/// email with [`extract_email`].
fn parse_address_list_for_bucket(v: Option<&Value>) -> Vec<String> {
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

/// Ingest a page of raw Gmail messages into the memory tree.
///
/// Each participant-bucket (sorted set of `from` ∪ `to` email addresses)
/// becomes one [`EmailThread`] handed to [`ingest_email`] which fans out
/// to the chunker + scorer + source tree downstream.
///
/// `source_id` = `"gmail:{participants}"` where participants is
/// `addr1|addr2|...` (sorted, deduped, lowercased). This groups all
/// correspondence between the same people into one path subtree.
///
/// Returns the total number of chunks written across all buckets so
/// callers can surface counts in logs / outcomes. Per-bucket errors are
/// logged and swallowed — one bad bucket should not abort the whole
/// page (the next sync re-fetches via the date-cursor).
pub async fn ingest_page_into_memory_tree(
    config: &Config,
    owner: &str,
    page_messages: &[Value],
) -> Result<usize> {
    if page_messages.is_empty() {
        return Ok(0);
    }
    let buckets = bucket_by_participants(page_messages);
    let mut total_chunks = 0usize;
    let mut total_buckets = 0usize;
    for (participants, raw_msgs) in &buckets {
        let messages: Vec<EmailMessage> = raw_msgs
            .iter()
            .filter_map(|raw| raw_to_email_message(raw))
            .collect();
        if messages.is_empty() {
            log::debug!(
                "[composio:gmail][ingest] skipping empty bucket participants_hash={}",
                redact(participants)
            );
            continue;
        }
        // source_id encodes participants so every unique conversation set
        // lands in its own path subtree.
        let source_id = format!("gmail:{}", participants);
        let thread_subject = pick_thread_subject(&messages);
        log::info!(
            "[composio:gmail][ingest] bucket participants_hash={} messages={} source_id_hash={}",
            redact(participants),
            messages.len(),
            redact(&source_id)
        );
        let thread = EmailThread {
            provider: GMAIL_PROVIDER.to_string(),
            thread_subject,
            messages,
        };
        let tags = DEFAULT_TAGS.iter().map(|s| (*s).to_string()).collect();
        match ingest_email(config, &source_id, owner, tags, thread).await {
            Ok(IngestResult { chunks_written, .. }) => {
                total_chunks += chunks_written;
                total_buckets += 1;
            }
            Err(e) => {
                log::warn!(
                    "[composio:gmail][ingest] ingest_email failed participants_hash={} source_id_hash={} err={:#}",
                    redact(participants),
                    redact(&source_id),
                    e
                );
            }
        }
    }
    log::info!(
        "[composio:gmail][ingest] page_done owner_hash={} buckets={total_buckets} chunks={total_chunks}",
        redact(owner)
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

    // ─── bucket_by_participants tests ─────────────────────────────────────────

    #[test]
    fn bidirectional_messages_bucket_together() {
        // alice→bob and bob→alice land in the same key "alice@x.com|bob@y.com".
        let msgs = vec![
            json!({
                "id": "m1",
                "from": "alice@x.com",
                "to": "bob@y.com",
                "subject": "Hi",
                "date": "2026-04-21T10:00:00Z",
                "markdown": "hi",
            }),
            json!({
                "id": "m2",
                "from": "bob@y.com",
                "to": "alice@x.com",
                "subject": "Re: Hi",
                "date": "2026-04-21T11:00:00Z",
                "markdown": "hey",
            }),
        ];
        let buckets = bucket_by_participants(&msgs);
        assert_eq!(buckets.len(), 1, "both messages must share one bucket");
        let key = buckets.keys().next().unwrap();
        assert_eq!(key, "alice@x.com|bob@y.com");
        assert_eq!(buckets[key].len(), 2);
        // Sorted ascending by date inside the bucket.
        assert_eq!(buckets[key][0].get("id").unwrap().as_str().unwrap(), "m1");
        assert_eq!(buckets[key][1].get("id").unwrap().as_str().unwrap(), "m2");
    }

    #[test]
    fn multi_recipient_bucket_key_sorted() {
        // from=alice, to=[bob, carol] → "alice@x.com|bob@y.com|carol@z.com"
        let msgs = vec![json!({
            "id": "m1",
            "from": "Alice <alice@x.com>",
            "to": ["bob@y.com", "carol@z.com"],
            "subject": "Group",
            "date": "2026-04-21T10:00:00Z",
            "markdown": "hey all",
        })];
        let buckets = bucket_by_participants(&msgs);
        let key = buckets.keys().next().unwrap();
        assert_eq!(key, "alice@x.com|bob@y.com|carol@z.com");
    }

    #[test]
    fn cc_field_ignored_in_bucket_key() {
        // from=alice, to=[bob], cc=[dave] → "alice@x.com|bob@y.com" (no dave).
        let msgs = vec![json!({
            "id": "m1",
            "from": "alice@x.com",
            "to": "bob@y.com",
            "cc": "dave@z.com",
            "subject": "CC test",
            "date": "2026-04-21T10:00:00Z",
            "markdown": "body",
        })];
        let buckets = bucket_by_participants(&msgs);
        let key = buckets.keys().next().unwrap();
        assert_eq!(
            key, "alice@x.com|bob@y.com",
            "CC must not appear in bucket key"
        );
    }

    #[test]
    fn solo_message_no_to_buckets_to_sender_only() {
        // from=alice, to=[] → "alice@x.com" (single participant).
        let msgs = vec![json!({
            "id": "m1",
            "from": "alice@x.com",
            "subject": "Draft",
            "date": "2026-04-21T10:00:00Z",
            "markdown": "draft body",
        })];
        let buckets = bucket_by_participants(&msgs);
        let key = buckets.keys().next().unwrap();
        assert_eq!(key, "alice@x.com");
    }

    #[test]
    fn empty_from_and_to_falls_back_to_orphan_bucket() {
        // A message with no parseable addresses gets its own orphan bucket
        // keyed by its id rather than collapsing everything into "unknown".
        let msgs = vec![json!({
            "id": "m1",
            "from": "",
            "subject": "x",
            "date": "2026-04-21T10:00:00Z",
            "markdown": "body",
        })];
        let buckets = bucket_by_participants(&msgs);
        assert_eq!(buckets.len(), 1, "must produce exactly one bucket");
        assert!(
            buckets.contains_key("orphan:m1"),
            "must fall back to orphan:<id>; got keys: {:?}",
            buckets.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn two_malformed_messages_with_different_ids_land_in_different_buckets() {
        // Two messages with unparseable from/to but different ids must not
        // collapse into the same "unknown" bucket — each gets its own orphan.
        let msgs = vec![
            json!({
                "id": "orphan_a",
                "from": "",
                "subject": "x",
                "date": "2026-04-21T10:00:00Z",
                "markdown": "body a",
            }),
            json!({
                "id": "orphan_b",
                "from": "",
                "subject": "y",
                "date": "2026-04-21T11:00:00Z",
                "markdown": "body b",
            }),
        ];
        let buckets = bucket_by_participants(&msgs);
        assert_eq!(
            buckets.len(),
            2,
            "each malformed message must have its own bucket; got: {:?}",
            buckets.keys().collect::<Vec<_>>()
        );
        assert!(buckets.contains_key("orphan:orphan_a"));
        assert!(buckets.contains_key("orphan:orphan_b"));
    }

    #[test]
    fn message_with_no_id_and_no_addresses_is_dropped() {
        // A message with no id AND no parseable addresses is silently dropped.
        let valid = json!({
            "id": "m_ok",
            "from": "alice@x.com",
            "subject": "ok",
            "date": "2026-04-21T10:00:00Z",
            "markdown": "ok",
        });
        let bad = json!({
            // no "id" field, no from/to
            "subject": "bad",
            "date": "2026-04-21T10:00:00Z",
            "markdown": "bad",
        });
        let msgs = vec![valid, bad];
        let buckets = bucket_by_participants(&msgs);
        // Only the valid message should produce a bucket.
        assert_eq!(buckets.len(), 1, "dropped message must not create a bucket");
        assert!(buckets.contains_key("alice@x.com"));
    }

    #[test]
    fn display_name_from_stripped_to_bare_email_in_key() {
        // "Alice <alice@x.com>" should yield bare "alice@x.com" in the key.
        let msgs = vec![json!({
            "id": "m1",
            "from": "Alice <alice@x.com>",
            "to": "Bob <bob@y.com>",
            "subject": "Hi",
            "date": "2026-04-21T10:00:00Z",
            "markdown": "hi",
        })];
        let buckets = bucket_by_participants(&msgs);
        let key = buckets.keys().next().unwrap();
        assert_eq!(key, "alice@x.com|bob@y.com");
    }

    #[test]
    fn no_threadid_field_does_not_affect_bucketing() {
        // threadId is completely ignored; two messages from the same participants
        // share one bucket even without threadId.
        let msgs = vec![
            json!({
                "id": "m1",
                "from": "noreply@github.com",
                "to": "sanil@x.com",
                "subject": "PR opened",
                "date": "2026-04-21T10:00:00Z",
                "markdown": "body1",
            }),
            json!({
                "id": "m2",
                "from": "noreply@github.com",
                "to": "sanil@x.com",
                "subject": "PR merged",
                "date": "2026-04-21T11:00:00Z",
                "markdown": "body2",
            }),
        ];
        let buckets = bucket_by_participants(&msgs);
        assert_eq!(buckets.len(), 1, "both messages must share one bucket");
        let bucket = buckets.values().next().unwrap();
        assert_eq!(bucket.len(), 2);
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
