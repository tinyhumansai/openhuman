//! Bucket → memory-tree ingest plumbing.
//!
//! This module owns the one-way conversion from our internal
//! [`Bucket`] type to [`ChatBatch`] + the call into
//! [`memory::tree::ingest::ingest_chat`]. It is intentionally free of
//! HTTP, timers, or state — those belong to [`super::engine`] — so the
//! canonicalisation path is easy to unit test.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use super::types::{Bucket, SlackChannel};
use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::canonicalize::chat::{ChatBatch, ChatMessage};
use crate::openhuman::memory::tree::ingest::{ingest_chat, IngestResult};

/// Platform identifier embedded in the canonical chat transcript header.
/// Matches the entry in `memory::tree::retrieval::source::PLATFORM_KINDS`.
pub const SLACK_PLATFORM: &str = "slack";

/// Tags attached to every Slack-ingested chunk. Keep this list stable —
/// retrieval callers filter on them.
pub const DEFAULT_TAGS: &[&str] = &["slack", "ingested"];

/// Convert a closed [`Bucket`] into a [`ChatBatch`] ready for
/// `memory::tree::ingest::ingest_chat`.
///
/// Empty buckets return `None` — upstream code should skip them rather
/// than ingest an empty payload. Bucket authorship is preserved by
/// using the Slack user ID (`author`) directly; the tree will not
/// attempt to resolve it to a display name (future enhancement could
/// plug in a `users.info` cache).
pub fn bucket_to_chat_batch(bucket: &Bucket, channel: &SlackChannel) -> Option<ChatBatch> {
    if bucket.messages.is_empty() {
        return None;
    }
    let messages = bucket
        .messages
        .iter()
        .map(|m| ChatMessage {
            author: if m.author.is_empty() {
                "unknown".to_string()
            } else {
                m.author.clone()
            },
            timestamp: m.timestamp,
            text: m.text.clone(),
            source_ref: Some(slack_archive_url(&channel.id, &m.ts_raw)),
        })
        .collect();

    Some(ChatBatch {
        platform: SLACK_PLATFORM.to_string(),
        channel_label: channel_label(channel),
        messages,
    })
}

/// Render a channel label for the canonical transcript header —
/// `"#eng"` for public channels, `"private:ops"` for private ones so
/// the retrieval side can distinguish them at a glance.
pub fn channel_label(channel: &SlackChannel) -> String {
    if channel.is_private {
        format!("private:{}", channel.name)
    } else {
        format!("#{}", channel.name)
    }
}

/// Permalink-shaped pointer to a Slack message. Not a real HTTPS URL —
/// we don't know the workspace hostname from the bot token alone — but
/// retains enough info for humans to reconstruct one.
fn slack_archive_url(channel_id: &str, ts_raw: &str) -> String {
    // Example: slack://archives/C012345/1714003200.000100
    format!("slack://archives/{channel_id}/{ts_raw}")
}

/// Ingest one bucket. Returns the tree's [`IngestResult`] so callers
/// can surface chunk counts in logs / RPC responses.
///
/// `connection_id` is the Composio connection (typically one Slack
/// workspace per connection). It becomes the `source_id` so all
/// channels' all buckets accumulate into ONE source tree per
/// workspace — letting the L0 buffer fill across many ingest calls
/// and eventually trigger a seal cascade. Chunk-id uniqueness across
/// buckets is preserved by `chunk_id` now hashing content.
pub async fn ingest_bucket(
    config: &Config,
    channel: &SlackChannel,
    bucket: &Bucket,
    owner: &str,
    connection_id: &str,
) -> Result<IngestResult> {
    let source_id = format!("slack:{connection_id}");
    let batch = match bucket_to_chat_batch(bucket, channel) {
        Some(b) => b,
        None => {
            log::debug!(
                "[slack_ingest][ops] skip empty bucket channel={} start={}",
                channel.id,
                bucket.start.to_rfc3339()
            );
            return Ok(IngestResult {
                source_id: source_id.clone(),
                chunks_written: 0,
                chunks_dropped: 0,
                chunk_ids: Vec::new(),
            });
        }
    };
    let tags = DEFAULT_TAGS.iter().map(|s| (*s).to_string()).collect();
    log::info!(
        "[slack_ingest][ops] ingest bucket channel={} start={} messages={} source_id={}",
        channel.id,
        bucket.start.to_rfc3339(),
        batch.messages.len(),
        source_id
    );
    ingest_chat(config, &source_id, owner, tags, batch)
        .await
        .with_context(|| {
            format!(
                "[slack_ingest][ops] ingest_chat failed channel={} source_id={}",
                channel.id, source_id
            )
        })
}

/// Unix seconds for an `end`-of-bucket cursor advance — used by the
/// engine to persist `last_synced_ts` after a successful ingest.
pub fn cursor_ts_for_flushed_bucket(end: DateTime<Utc>) -> i64 {
    end.timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::slack_ingestion::types::SlackMessage;
    use chrono::TimeZone;

    fn sample_channel() -> SlackChannel {
        SlackChannel {
            id: "C0123456".into(),
            name: "eng".into(),
            is_private: false,
        }
    }

    fn sample_message(ts_secs: i64, author: &str, text: &str) -> SlackMessage {
        SlackMessage {
            channel_id: "C0123456".into(),
            author: author.into(),
            text: text.into(),
            timestamp: Utc.timestamp_opt(ts_secs, 0).single().unwrap(),
            ts_raw: format!("{ts_secs}.000000"),
            thread_ts: None,
        }
    }

    #[test]
    fn bucket_to_chat_batch_builds_expected_payload() {
        let channel = sample_channel();
        let start = Utc.with_ymd_and_hms(2026, 4, 25, 6, 0, 0).unwrap();
        let end = start + chrono::Duration::hours(6);
        let bucket = Bucket {
            start,
            end,
            messages: vec![
                sample_message(start.timestamp() + 60, "U1", "hello"),
                sample_message(start.timestamp() + 120, "U2", "world"),
            ],
        };
        let batch = bucket_to_chat_batch(&bucket, &channel).unwrap();
        assert_eq!(batch.platform, "slack");
        assert_eq!(batch.channel_label, "#eng");
        assert_eq!(batch.messages.len(), 2);
        assert_eq!(batch.messages[0].author, "U1");
        assert_eq!(batch.messages[0].text, "hello");
        assert!(batch.messages[0]
            .source_ref
            .as_deref()
            .unwrap()
            .starts_with("slack://archives/C0123456/"));
    }

    #[test]
    fn bucket_to_chat_batch_none_for_empty_messages() {
        let channel = sample_channel();
        let start = Utc.with_ymd_and_hms(2026, 4, 25, 6, 0, 0).unwrap();
        let bucket = Bucket {
            start,
            end: start + chrono::Duration::hours(6),
            messages: vec![],
        };
        assert!(bucket_to_chat_batch(&bucket, &channel).is_none());
    }

    #[test]
    fn channel_label_distinguishes_private() {
        let pub_ch = sample_channel();
        let priv_ch = SlackChannel {
            id: "G1".into(),
            name: "ops".into(),
            is_private: true,
        };
        assert_eq!(channel_label(&pub_ch), "#eng");
        assert_eq!(channel_label(&priv_ch), "private:ops");
    }

    #[test]
    fn missing_author_defaults_to_unknown() {
        let channel = sample_channel();
        let start = Utc.with_ymd_and_hms(2026, 4, 25, 6, 0, 0).unwrap();
        let bucket = Bucket {
            start,
            end: start + chrono::Duration::hours(6),
            messages: vec![sample_message(start.timestamp() + 30, "", "anon")],
        };
        let batch = bucket_to_chat_batch(&bucket, &channel).unwrap();
        assert_eq!(batch.messages[0].author, "unknown");
    }

    #[test]
    fn slack_archive_url_format_is_stable() {
        let url = slack_archive_url("C1", "1714003200.000100");
        assert_eq!(url, "slack://archives/C1/1714003200.000100");
    }

    #[test]
    fn cursor_ts_uses_bucket_end() {
        let end = Utc.with_ymd_and_hms(2026, 4, 25, 12, 0, 0).unwrap();
        assert_eq!(cursor_ts_for_flushed_bucket(end), end.timestamp());
    }
}
