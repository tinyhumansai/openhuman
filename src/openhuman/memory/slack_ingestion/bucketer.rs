//! 6-hour UTC-aligned bucketing for Slack messages.
//!
//! Buckets have fixed wall-clock starts at `00:00`, `06:00`, `12:00`, and
//! `18:00` UTC each day. A message's bucket is determined solely by its
//! timestamp — the engine never decides bucketing based on order of
//! arrival or polling cadence, so the same message always lands in the
//! same bucket regardless of when it's seen.
//!
//! ## Why closed-bucket-only?
//!
//! Each ingested chunk becomes a *leaf* in the source tree via
//! `append_leaf` (see `memory::tree::source_tree::bucket_seal`). Leaves
//! participate in a token-budget seal cascade — once appended, they
//! cannot be cleanly retracted from already-sealed summary nodes
//! upstream. So we only ingest a bucket once, *after* it has closed
//! (window end + grace period is past).
//!
//! The `source_id` convention (`"slack:<channel>:<bucket_start_epoch>"`)
//! plus deterministic chunk IDs give us idempotent re-ingest for late
//! edits in a future iteration without breaking that invariant.

use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};
use std::collections::BTreeMap;

use super::types::{Bucket, SlackMessage};

/// Width of each bucket. Changing this changes the ingest granularity
/// *and* invalidates existing `source_id`s, so treat it as a schema
/// constant — not a runtime tunable.
pub const BUCKET_HOURS: u32 = 6;

/// Safety window after a bucket's nominal end before we consider it
/// closed. Protects against clock skew, slow API polls, or messages
/// that arrive via `conversations.history` a few minutes after they
/// were sent.
pub const GRACE_PERIOD: Duration = Duration::minutes(15);

/// Round a timestamp down to its 6-hour UTC bucket start.
///
/// For `2026-04-25T13:42:07Z` this returns `2026-04-25T12:00:00Z`.
pub fn bucket_start_for(ts: DateTime<Utc>) -> DateTime<Utc> {
    let hour = ts.hour();
    let bucket_hour = (hour / BUCKET_HOURS) * BUCKET_HOURS;
    Utc.with_ymd_and_hms(ts.year(), ts.month(), ts.day(), bucket_hour, 0, 0)
        .single()
        .expect("00/06/12/18 are always valid wall-clock hours in UTC")
}

/// Exclusive bucket end — `start + BUCKET_HOURS`.
pub fn bucket_end_for(start: DateTime<Utc>) -> DateTime<Utc> {
    start + Duration::hours(BUCKET_HOURS as i64)
}

/// Stable identifier for a channel's bucket, used as the memory-tree
/// `source_id`. Uses Unix seconds so the id is compact, comparable, and
/// independent of the caller's locale.
pub fn source_id_for(channel_id: &str, bucket_start: DateTime<Utc>) -> String {
    format!("slack:{channel_id}:{}", bucket_start.timestamp())
}

/// Partition an in-memory buffer into closed buckets (ready to ingest)
/// and the messages that remain in still-open buckets.
///
/// A bucket is *closed* when `bucket_end + GRACE_PERIOD <= now`. Messages
/// in closed buckets are grouped into [`Bucket`] values ordered by start.
/// Messages in open buckets flow back into `remaining` and will be
/// re-evaluated on a later tick when their window closes.
///
/// Empty buckets are never produced — if no messages fall into a range
/// there is no [`Bucket`] for it.
pub fn split_closed(
    buffer: Vec<SlackMessage>,
    now: DateTime<Utc>,
    grace: Duration,
) -> (Vec<Bucket>, Vec<SlackMessage>) {
    let mut by_bucket: BTreeMap<DateTime<Utc>, Vec<SlackMessage>> = BTreeMap::new();
    for msg in buffer {
        let start = bucket_start_for(msg.timestamp);
        by_bucket.entry(start).or_default().push(msg);
    }

    let mut closed = Vec::new();
    let mut remaining = Vec::new();
    for (start, mut messages) in by_bucket {
        let end = bucket_end_for(start);
        if end + grace <= now {
            // Preserve chronological order within the bucket.
            messages.sort_by_key(|m| m.timestamp);
            closed.push(Bucket {
                start,
                end,
                messages,
            });
        } else {
            remaining.extend(messages);
        }
    }
    (closed, remaining)
}

/// Enumerate every 6-hour bucket start in `[from, until)` — used by the
/// backfill path to walk the last N days in order.
pub fn buckets_between(from: DateTime<Utc>, until: DateTime<Utc>) -> Vec<DateTime<Utc>> {
    if until <= from {
        return Vec::new();
    }
    let mut starts = Vec::new();
    let mut cursor = bucket_start_for(from);
    while cursor < until {
        starts.push(cursor);
        cursor = bucket_end_for(cursor);
    }
    starts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(year: i32, month: u32, day: u32, hour: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, min, 0)
            .single()
            .unwrap()
    }

    fn msg(channel: &str, timestamp: DateTime<Utc>, body: &str) -> SlackMessage {
        SlackMessage {
            channel_id: channel.into(),
            author: "U1".into(),
            text: body.into(),
            timestamp,
            ts_raw: format!("{}.000000", timestamp.timestamp()),
            thread_ts: None,
        }
    }

    #[test]
    fn bucket_start_rounds_down_to_six_hour_boundary() {
        assert_eq!(
            bucket_start_for(ts(2026, 4, 25, 0, 0)),
            ts(2026, 4, 25, 0, 0)
        );
        assert_eq!(
            bucket_start_for(ts(2026, 4, 25, 5, 59)),
            ts(2026, 4, 25, 0, 0)
        );
        assert_eq!(
            bucket_start_for(ts(2026, 4, 25, 6, 0)),
            ts(2026, 4, 25, 6, 0)
        );
        assert_eq!(
            bucket_start_for(ts(2026, 4, 25, 13, 42)),
            ts(2026, 4, 25, 12, 0)
        );
        assert_eq!(
            bucket_start_for(ts(2026, 4, 25, 23, 59)),
            ts(2026, 4, 25, 18, 0)
        );
    }

    #[test]
    fn bucket_end_is_start_plus_six_hours() {
        let start = ts(2026, 4, 25, 6, 0);
        assert_eq!(bucket_end_for(start), ts(2026, 4, 25, 12, 0));
    }

    #[test]
    fn source_id_is_deterministic_and_channel_scoped() {
        let start = ts(2026, 4, 25, 12, 0);
        assert_eq!(
            source_id_for("C0123456", start),
            format!("slack:C0123456:{}", start.timestamp())
        );
        assert_ne!(
            source_id_for("C0123456", start),
            source_id_for("C9999999", start),
        );
    }

    #[test]
    fn split_closed_keeps_open_bucket_in_remaining() {
        let now = ts(2026, 4, 25, 7, 0); // bucket [06:00, 12:00) still open
        let buf = vec![msg("C1", ts(2026, 4, 25, 6, 30), "hi")];
        let (closed, remaining) = split_closed(buf, now, GRACE_PERIOD);
        assert!(closed.is_empty());
        assert_eq!(remaining.len(), 1);
    }

    #[test]
    fn split_closed_emits_fully_closed_bucket() {
        // Bucket [00:00, 06:00) closes at 06:15 with grace; now is 07:00.
        let now = ts(2026, 4, 25, 7, 0);
        let buf = vec![
            msg("C1", ts(2026, 4, 25, 1, 0), "a"),
            msg("C1", ts(2026, 4, 25, 3, 30), "b"),
        ];
        let (closed, remaining) = split_closed(buf, now, GRACE_PERIOD);
        assert_eq!(closed.len(), 1);
        assert_eq!(remaining.len(), 0);
        assert_eq!(closed[0].start, ts(2026, 4, 25, 0, 0));
        assert_eq!(closed[0].end, ts(2026, 4, 25, 6, 0));
        assert_eq!(closed[0].messages.len(), 2);
        // Sorted chronologically inside the bucket.
        assert!(closed[0].messages[0].timestamp < closed[0].messages[1].timestamp);
    }

    #[test]
    fn split_closed_respects_grace_boundary() {
        // Bucket ends at 06:00. With 15-min grace, it closes at 06:15.
        let just_before = ts(2026, 4, 25, 6, 14);
        let just_after = ts(2026, 4, 25, 6, 15);
        let buf = || vec![msg("C1", ts(2026, 4, 25, 3, 0), "x")];

        let (closed_before, _) = split_closed(buf(), just_before, GRACE_PERIOD);
        assert!(closed_before.is_empty(), "still within grace window");

        let (closed_after, remaining) = split_closed(buf(), just_after, GRACE_PERIOD);
        assert_eq!(closed_after.len(), 1, "grace expired exactly at 06:15");
        assert!(remaining.is_empty());
    }

    #[test]
    fn split_closed_handles_mixed_buckets() {
        let now = ts(2026, 4, 25, 13, 0);
        let buf = vec![
            msg("C1", ts(2026, 4, 25, 1, 0), "closed bucket 1"),
            msg("C1", ts(2026, 4, 25, 7, 0), "closed bucket 2"),
            msg("C1", ts(2026, 4, 25, 12, 5), "open bucket"),
        ];
        let (closed, remaining) = split_closed(buf, now, GRACE_PERIOD);
        assert_eq!(closed.len(), 2);
        assert_eq!(remaining.len(), 1);
        assert_eq!(closed[0].start, ts(2026, 4, 25, 0, 0));
        assert_eq!(closed[1].start, ts(2026, 4, 25, 6, 0));
    }

    #[test]
    fn split_closed_empty_buffer_is_noop() {
        let (closed, remaining) = split_closed(vec![], ts(2026, 4, 25, 12, 0), GRACE_PERIOD);
        assert!(closed.is_empty());
        assert!(remaining.is_empty());
    }

    #[test]
    fn buckets_between_walks_six_hour_starts() {
        let from = ts(2026, 4, 25, 2, 0); // snaps to 00:00
        let until = ts(2026, 4, 26, 0, 0);
        let starts = buckets_between(from, until);
        assert_eq!(starts.len(), 4);
        assert_eq!(starts[0], ts(2026, 4, 25, 0, 0));
        assert_eq!(starts[1], ts(2026, 4, 25, 6, 0));
        assert_eq!(starts[2], ts(2026, 4, 25, 12, 0));
        assert_eq!(starts[3], ts(2026, 4, 25, 18, 0));
    }

    #[test]
    fn buckets_between_empty_when_range_reversed() {
        let starts = buckets_between(ts(2026, 4, 26, 0, 0), ts(2026, 4, 25, 0, 0));
        assert!(starts.is_empty());
    }
}
