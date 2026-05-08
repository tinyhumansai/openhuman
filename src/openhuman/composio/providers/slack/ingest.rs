//! Slack → memory tree ingest plumbing.
//!
//! Owns the conversion from a page of [`SlackMessage`]s (post-processed
//! and enriched by [`super::sync`]) into per-channel [`ChatBatch`]es and
//! drives [`memory::tree::ingest::ingest_chat`] per message.
//!
//! ## Source-id scope
//!
//! Source id is `slack:{connection_id}` (workspace-wide), NOT per-channel.
//! Channel label lives in [`ChatBatch.channel_label`] for display in the
//! tree; all channels in one Slack workspace accumulate into one source
//! tree so the L0 buffer fills across many ingest calls and the seal
//! cascade fires at the right cadence.
//!
//! ## Per-message ingest
//!
//! We call `ingest_chat` once per message with a single-message
//! `ChatBatch`, then `set_chunk_raw_refs` to link the resulting chunk to
//! its raw archive entry. This gives 1:1 chunk-to-raw-file mapping that
//! mirrors the Gmail per-account path.
//!
//! ## Idempotency
//!
//! Chunk IDs are content-hashed inside the memory tree, so re-ingesting
//! a previously-seen message is an UPSERT — no duplicates across syncs.

use std::collections::BTreeMap;

use anyhow::Result;

use super::types::SlackMessage;
use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::canonicalize::chat::{ChatBatch, ChatMessage};
use crate::openhuman::memory::tree::content_store::raw::{
    self as raw_store, raw_rel_path, RawItem, RawKind,
};
use crate::openhuman::memory::tree::ingest::ingest_chat;
use crate::openhuman::memory::tree::store::{set_chunk_raw_refs, RawRef};
use crate::openhuman::memory::tree::util::redact::redact;

/// Platform identifier embedded in the canonical chat transcript header.
/// Matches the value `memory::tree::retrieval::source::PLATFORM_KINDS` expects.
pub const SLACK_PLATFORM: &str = "slack";

/// Tags attached to every Slack-ingested chunk. Stable list — retrieval
/// callers filter on these.
pub const DEFAULT_TAGS: &[&str] = &["slack", "ingested"];

/// Group a page of messages by `channel_id`. Each group is sorted
/// ascending by timestamp so ingest calls read chronologically.
///
/// Analogous to Gmail's `bucket_by_participants`, but trivial for Slack:
/// every message already carries its channel_id.
pub(crate) fn bucket_by_channel<'a>(
    msgs: &'a [SlackMessage],
) -> BTreeMap<String, Vec<&'a SlackMessage>> {
    let mut out: BTreeMap<String, Vec<&'a SlackMessage>> = BTreeMap::new();
    for m in msgs {
        out.entry(m.channel_id.clone()).or_default().push(m);
    }
    for bucket in out.values_mut() {
        bucket.sort_by_key(|m| m.timestamp);
    }
    out
}

/// Render a channel label for the canonical transcript header.
///
/// Public channels become `"#eng"`; private channels become
/// `"private:ops"` so the retrieval side can distinguish them at a
/// glance.
pub(crate) fn channel_label(channel_name: &str, is_private: bool) -> String {
    if is_private {
        format!("private:{channel_name}")
    } else {
        format!("#{channel_name}")
    }
}

/// Convert a [`SlackMessage`] into a [`ChatMessage`] for the memory tree.
///
/// Author falls back to `"unknown"` when the resolved name is empty.
/// `source_ref` prefers the HTTPS `permalink` from the Composio response;
/// when absent it falls back to the stable `slack://archives/…` scheme.
pub(crate) fn slack_message_to_chat_message(m: &SlackMessage) -> ChatMessage {
    let author = if m.author.is_empty() {
        "unknown".to_string()
    } else {
        m.author.clone()
    };
    let source_ref = m
        .permalink
        .clone()
        .or_else(|| Some(format!("slack://archives/{}/{}", m.channel_id, m.ts_raw)));
    ChatMessage {
        author,
        timestamp: m.timestamp,
        text: m.text.clone(),
        source_ref,
    }
}

/// Ingest a page of Slack messages into the memory tree.
///
/// Messages are grouped by channel_id and ingested one at a time via
/// `ingest_chat` (per-message mode). Each successful ingest links the
/// returned chunk(s) to a raw archive entry via `set_chunk_raw_refs` so
/// `read_chunk_body` can reconstruct full bodies without duplicating
/// bytes in the SQL `content` column.
///
/// Per-channel errors are logged and swallowed — one bad message should
/// not abort the whole page (the next sync re-fetches via the
/// date-cursor).
///
/// Returns the total number of chunks written.
pub async fn ingest_page_into_memory_tree(
    config: &Config,
    owner: &str,
    connection_id: &str,
    page_messages: &[SlackMessage],
) -> Result<usize> {
    if page_messages.is_empty() {
        return Ok(0);
    }

    let source_id = format!("slack:{connection_id}");

    // Best-effort raw archive — written before chunking so a chunker bug
    // doesn't block capturing the source bytes.
    if let Err(e) = write_raw_archive(config, &source_id, page_messages) {
        log::warn!(
            "[composio:slack][ingest] raw archive write failed source_id_hash={} err={:#}",
            redact(&source_id),
            e
        );
    }

    let total_chunks = ingest_per_message(config, &source_id, owner, page_messages).await;

    log::info!(
        "[composio:slack][ingest] page_done owner_hash={} connection_hash={} chunks={total_chunks}",
        redact(owner),
        redact(connection_id),
    );
    Ok(total_chunks)
}

/// Per-message ingest: one `ingest_chat` call per Slack message.
///
/// Each call produces 1 chunk for normal messages or N chunks for oversize
/// messages. After the ingest we tag every resulting chunk with a
/// [`RawRef`] pointing at the raw archive file written during
/// [`write_raw_archive`], so `read_chunk_body` can reconstruct full bodies
/// without duplicating bytes in the SQL `content` column.
async fn ingest_per_message(
    config: &Config,
    source_id: &str,
    owner: &str,
    page_messages: &[SlackMessage],
) -> usize {
    let mut total_chunks = 0usize;

    for m in page_messages {
        if m.text.trim().is_empty() {
            log::debug!(
                "[composio:slack][ingest] skipping empty-body message ts_raw={}",
                m.ts_raw
            );
            continue;
        }

        let ts_ms = m.timestamp.timestamp_millis();
        let raw_path = raw_rel_path(source_id, RawKind::Chat, ts_ms, &m.ts_raw);

        let chat_message = slack_message_to_chat_message(m);
        let label = channel_label(&m.channel_name, m.is_private);
        let batch = ChatBatch {
            platform: SLACK_PLATFORM.to_string(),
            channel_label: label,
            messages: vec![chat_message],
        };
        let tags = DEFAULT_TAGS.iter().map(|s| (*s).to_string()).collect();

        match ingest_chat(config, source_id, owner, tags, batch).await {
            Ok(result) => {
                total_chunks += result.chunks_written;
                let refs = vec![RawRef {
                    path: raw_path.clone(),
                    start: 0,
                    end: None,
                }];
                for chunk_id in &result.chunk_ids {
                    if let Err(e) = set_chunk_raw_refs(config, chunk_id, &refs) {
                        log::warn!(
                            "[composio:slack][ingest] set_chunk_raw_refs failed chunk_id={} err={:#}",
                            chunk_id,
                            e
                        );
                    }
                }
                log::debug!(
                    "[composio:slack][ingest] ingested message ts_raw={} chunks={}",
                    m.ts_raw,
                    result.chunks_written
                );
            }
            Err(e) => {
                log::warn!(
                    "[composio:slack][ingest] per-message ingest_chat failed ts_raw_hash={} err={:#}",
                    redact(&m.ts_raw),
                    e
                );
            }
        }
    }

    total_chunks
}

/// Mirror a page of Slack messages into the on-disk raw archive.
///
/// Files land under `<content_root>/raw/<source_slug>/chats/<ts_ms>_<ts_raw>.md`
/// — the `chats/` subdir is selected automatically by [`RawKind::Chat`]
/// (see `content_store::raw`).
/// Each file gets a small metadata header (channel, author, date) followed
/// by the message body so the file is self-describing when opened
/// standalone in Obsidian or a text editor.
///
/// Messages with an empty body are skipped — they'd produce
/// zero-content files. Messages without a parseable timestamp produce
/// non-stable filenames so they are also skipped.
fn write_raw_archive(config: &Config, source_id: &str, page: &[SlackMessage]) -> Result<usize> {
    let content_root = config.memory_tree_content_root();
    let mut bodies: Vec<(String, i64, String)> = Vec::with_capacity(page.len());

    for m in page {
        let body = m.text.trim();
        if body.is_empty() {
            log::debug!(
                "[composio:slack][raw] empty body ts_raw={} — skipping",
                m.ts_raw
            );
            continue;
        }

        let label = channel_label(&m.channel_name, m.is_private);
        let ts_ms = m.timestamp.timestamp_millis();
        let date_str = m.timestamp.to_rfc3339();

        let mut composed = String::with_capacity(body.len() + 256);
        composed.push_str(&format!("**Channel:** {label}\n"));
        composed.push_str(&format!("**Author:** {}\n", m.author));
        composed.push_str(&format!("**Date:** {date_str}\n\n"));
        composed.push_str(body);

        bodies.push((m.ts_raw.clone(), ts_ms, composed));
    }

    let items: Vec<RawItem<'_>> = bodies
        .iter()
        .map(|(uid, ts, md)| RawItem {
            uid: uid.as_str(),
            created_at_ms: *ts,
            markdown: md.as_str(),
            kind: RawKind::Chat,
        })
        .collect();

    let n = raw_store::write_raw_items(&content_root, source_id, &items)?;
    log::debug!(
        "[composio:slack][raw] archived {n} messages source_id_hash={}",
        redact(source_id)
    );
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(secs: i64) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc.timestamp_opt(secs, 0).single().unwrap()
    }

    fn make_message(
        channel_id: &str,
        channel_name: &str,
        is_private: bool,
        secs: i64,
    ) -> SlackMessage {
        SlackMessage {
            channel_id: channel_id.to_string(),
            channel_name: channel_name.to_string(),
            is_private,
            author: "alice".to_string(),
            author_id: "U001".to_string(),
            text: "hello".to_string(),
            timestamp: ts(secs),
            ts_raw: format!("{secs}.000000"),
            thread_ts: None,
            permalink: None,
        }
    }

    // ─── bucket_by_channel ────────────────────────────────────────────────────

    #[test]
    fn bucket_by_channel_groups_messages() {
        let msgs = vec![
            make_message("C1", "eng", false, 1000),
            make_message("C2", "ops", false, 1100),
            make_message("C1", "eng", false, 1200),
        ];
        let buckets = bucket_by_channel(&msgs);
        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets["C1"].len(), 2);
        assert_eq!(buckets["C2"].len(), 1);
    }

    #[test]
    fn bucket_by_channel_sorts_chronologically() {
        let msgs = vec![
            make_message("C1", "eng", false, 2000),
            make_message("C1", "eng", false, 1000),
        ];
        let buckets = bucket_by_channel(&msgs);
        let eng = &buckets["C1"];
        assert_eq!(eng[0].timestamp, ts(1000));
        assert_eq!(eng[1].timestamp, ts(2000));
    }

    // ─── channel_label ────────────────────────────────────────────────────────

    #[test]
    fn channel_label_distinguishes_private() {
        assert_eq!(channel_label("eng", false), "#eng");
        assert_eq!(channel_label("ops", true), "private:ops");
    }

    // ─── slack_message_to_chat_message ────────────────────────────────────────

    #[test]
    fn slack_message_to_chat_message_falls_back_to_unknown_author() {
        let m = SlackMessage {
            channel_id: "C1".into(),
            channel_name: "eng".into(),
            is_private: false,
            author: "".into(),
            author_id: "U001".into(),
            text: "hi".into(),
            timestamp: ts(1000),
            ts_raw: "1000.000000".into(),
            thread_ts: None,
            permalink: None,
        };
        let cm = slack_message_to_chat_message(&m);
        assert_eq!(cm.author, "unknown");
    }

    #[test]
    fn slack_message_to_chat_message_uses_permalink_when_present() {
        let m = SlackMessage {
            channel_id: "C1".into(),
            channel_name: "eng".into(),
            is_private: false,
            author: "alice".into(),
            author_id: "U001".into(),
            text: "hi".into(),
            timestamp: ts(1000),
            ts_raw: "1000.000000".into(),
            thread_ts: None,
            permalink: Some("https://myworkspace.slack.com/archives/C1/p1000000000".into()),
        };
        let cm = slack_message_to_chat_message(&m);
        assert_eq!(
            cm.source_ref.as_deref(),
            Some("https://myworkspace.slack.com/archives/C1/p1000000000")
        );
    }

    #[test]
    fn slack_message_to_chat_message_falls_back_to_archive_url() {
        let m = SlackMessage {
            channel_id: "C1".into(),
            channel_name: "eng".into(),
            is_private: false,
            author: "alice".into(),
            author_id: "U001".into(),
            text: "hi".into(),
            timestamp: ts(1000),
            ts_raw: "1000.000000".into(),
            thread_ts: None,
            permalink: None,
        };
        let cm = slack_message_to_chat_message(&m);
        assert_eq!(
            cm.source_ref.as_deref(),
            Some("slack://archives/C1/1000.000000")
        );
    }
}
