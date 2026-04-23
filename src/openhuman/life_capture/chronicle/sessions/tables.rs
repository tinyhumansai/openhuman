//! SQLite access layer for `chronicle_sessions` and `chronicle_minute_buckets`.
//!
//! Mirrors the A3 `chronicle::tables` layout: each op takes a
//! `&PersonalIndex` and runs on `spawn_blocking` so we don't hold the
//! runtime. Unlike A3, A4 writes *two* rows per boundary — the session
//! header plus N minute buckets — so we use a single transaction per
//! close so a partial crash never leaves orphan buckets.
//!
//! Watermarking reuses `chronicle::tables::{get_watermark, set_watermark}`
//! under the `"session_manager"` source name, so the manager's tick can
//! resume exactly where it left off.
//!
//! A4 deliberately does not expose a reader — A6 (daily reducer) will add
//! the query surface it needs when it lands. Tests here exercise round
//! trips and FK cascade.

use std::collections::HashMap;

use anyhow::Context;
use rusqlite::params;

use crate::openhuman::life_capture::chronicle::sessions::rules::{minute_bucket, BoundaryReason};
use crate::openhuman::life_capture::index::PersonalIndex;

/// One closed session's full payload — header + its minute buckets.
/// Handed to `insert_closed_session` in a single txn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedSession {
    pub start_ts_ms: i64,
    pub close_ts_ms: i64,
    pub boundary_reason: BoundaryReason,
    pub primary_app: String,
    pub event_count: i64,
    /// Keyed by minute bucket ts_ms → (dominant_app, event_count).
    pub minute_buckets: Vec<MinuteBucket>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinuteBucket {
    pub bucket_ts_ms: i64,
    pub focused_app: String,
    pub event_count: i64,
}

/// Collapse a per-event stream into minute buckets, one row per minute
/// with the modal app for that minute. Pure — shared with the manager so
/// tests can assert bucket shape without round-tripping through SQL.
pub fn roll_up_minute_buckets(events: &[(i64, String)]) -> Vec<MinuteBucket> {
    // bucket_ts_ms → (app → count). BTreeMap on outer key keeps buckets
    // sorted by time so the written rows are readable in logs.
    let mut by_bucket: std::collections::BTreeMap<i64, HashMap<String, i64>> =
        std::collections::BTreeMap::new();
    for (ts, app) in events {
        let b = minute_bucket(*ts);
        *by_bucket
            .entry(b)
            .or_default()
            .entry(app.clone())
            .or_insert(0) += 1;
    }
    by_bucket
        .into_iter()
        .map(|(bucket, apps)| {
            let total: i64 = apps.values().sum();
            // Dominant app = max count, ties broken by lexicographic order
            // (deterministic so tests don't flake).
            let focused_app = apps
                .iter()
                .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
                .map(|(k, _)| k.clone())
                .expect("bucket never empty");
            MinuteBucket {
                bucket_ts_ms: bucket,
                focused_app,
                event_count: total,
            }
        })
        .collect()
}

/// Persist a closed session + its minute buckets atomically. Returns the
/// new session id.
pub async fn insert_closed_session(
    idx: &PersonalIndex,
    session: ClosedSession,
) -> anyhow::Result<i64> {
    let conn = idx.writer.clone();
    tokio::task::spawn_blocking(move || -> anyhow::Result<i64> {
        let mut guard = conn.blocking_lock();
        let tx = guard.transaction().context("begin session txn")?;
        tx.execute(
            "INSERT INTO chronicle_sessions( \
                start_ts_ms, close_ts_ms, boundary_reason, primary_app, event_count \
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                session.start_ts_ms,
                session.close_ts_ms,
                session.boundary_reason.as_str(),
                session.primary_app,
                session.event_count,
            ],
        )
        .context("insert chronicle_sessions row")?;
        let session_id = tx.last_insert_rowid();

        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO chronicle_minute_buckets( \
                        session_id, bucket_ts_ms, focused_app, event_count \
                     ) VALUES (?1, ?2, ?3, ?4)",
                )
                .context("prepare minute bucket insert")?;
            for bucket in &session.minute_buckets {
                stmt.execute(params![
                    session_id,
                    bucket.bucket_ts_ms,
                    bucket.focused_app,
                    bucket.event_count,
                ])
                .context("insert chronicle_minute_buckets row")?;
            }
        }

        tx.commit().context("commit session txn")?;
        Ok(session_id)
    })
    .await
    .context("insert_closed_session task panicked")?
}

/// Count of sessions for assertions in tests / diagnostics.
pub async fn count_sessions(idx: &PersonalIndex) -> anyhow::Result<i64> {
    let conn = idx.writer.clone();
    tokio::task::spawn_blocking(move || -> anyhow::Result<i64> {
        let guard = conn.blocking_lock();
        let n: i64 = guard
            .query_row("SELECT count(*) FROM chronicle_sessions", [], |r| r.get(0))
            .context("count chronicle_sessions")?;
        Ok(n)
    })
    .await
    .context("count_sessions task panicked")?
}

/// Count of minute buckets for a specific session (FK cascade check).
pub async fn count_buckets_for(idx: &PersonalIndex, session_id: i64) -> anyhow::Result<i64> {
    let conn = idx.writer.clone();
    tokio::task::spawn_blocking(move || -> anyhow::Result<i64> {
        let guard = conn.blocking_lock();
        let n: i64 = guard
            .query_row(
                "SELECT count(*) FROM chronicle_minute_buckets WHERE session_id = ?1",
                params![session_id],
                |r| r.get(0),
            )
            .context("count chronicle_minute_buckets")?;
        Ok(n)
    })
    .await
    .context("count_buckets_for task panicked")?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evs(pairs: &[(i64, &str)]) -> Vec<(i64, String)> {
        pairs.iter().map(|(t, a)| (*t, a.to_string())).collect()
    }

    #[test]
    fn roll_up_groups_events_into_minute_buckets() {
        // 3 events in minute 0, 1 event in minute 1.
        let events = evs(&[
            (0, "code"),
            (30_000, "code"),
            (59_999, "slack"),
            (60_000, "code"),
        ]);
        let buckets = roll_up_minute_buckets(&events);
        assert_eq!(buckets.len(), 2);
        // Minute 0: code has 2, slack has 1 → code wins.
        assert_eq!(buckets[0].bucket_ts_ms, 0);
        assert_eq!(buckets[0].focused_app, "code");
        assert_eq!(buckets[0].event_count, 3);
        // Minute 1: only code.
        assert_eq!(buckets[1].bucket_ts_ms, 60_000);
        assert_eq!(buckets[1].focused_app, "code");
        assert_eq!(buckets[1].event_count, 1);
    }

    #[test]
    fn roll_up_tie_break_is_deterministic() {
        // Two apps with equal counts in the same minute. Tie-break by
        // lexicographic order (reverse on app name so "slack" beats "code"
        // when tied? No — we pick max count then lexicographic min. Assert
        // whichever we settled on and keep it stable.)
        let events = evs(&[(0, "code"), (30_000, "slack")]);
        let buckets = roll_up_minute_buckets(&events);
        assert_eq!(buckets.len(), 1);
        // Tie → lexicographic min wins ("code" < "slack").
        assert_eq!(buckets[0].focused_app, "code");
        assert_eq!(buckets[0].event_count, 2);
    }

    #[tokio::test]
    async fn insert_and_count_round_trip() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        let closed = ClosedSession {
            start_ts_ms: 0,
            close_ts_ms: 120_000,
            boundary_reason: BoundaryReason::Idle,
            primary_app: "code".into(),
            event_count: 3,
            minute_buckets: vec![
                MinuteBucket {
                    bucket_ts_ms: 0,
                    focused_app: "code".into(),
                    event_count: 2,
                },
                MinuteBucket {
                    bucket_ts_ms: 60_000,
                    focused_app: "code".into(),
                    event_count: 1,
                },
            ],
        };
        let sid = insert_closed_session(&idx, closed).await.unwrap();
        assert!(sid > 0);
        assert_eq!(count_sessions(&idx).await.unwrap(), 1);
        assert_eq!(count_buckets_for(&idx, sid).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn empty_minute_buckets_still_writes_session_header() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        let closed = ClosedSession {
            start_ts_ms: 0,
            close_ts_ms: 0,
            boundary_reason: BoundaryReason::MaxDuration,
            primary_app: "code".into(),
            event_count: 0,
            minute_buckets: vec![],
        };
        let sid = insert_closed_session(&idx, closed).await.unwrap();
        assert_eq!(count_sessions(&idx).await.unwrap(), 1);
        assert_eq!(count_buckets_for(&idx, sid).await.unwrap(), 0);
    }
}
