//! Session manager: consumes chronicle_events past the watermark, applies
//! boundary rules, and persists closed sessions + minute buckets.
//!
//! Pull model (not event bus) — A3 writes chronicle rows synchronously
//! without emitting on the bus, so the manager periodically reads new
//! rows in ts_ms order past its own watermark. Watermark is advanced
//! only to the `ts_ms` of the last event folded into a *closed* session
//! so a crash mid-session replays those events on the next tick and
//! regenerates the same boundaries deterministically.
//!
//! The open session lives in-memory on the manager struct. A restart
//! loses the open-session header (not the underlying events) — A6 can
//! reconstruct from scratch, or a future slice can persist it. Out of
//! scope for A4.

use std::sync::Arc;

use anyhow::Context;
use rusqlite::params;
use tokio::sync::Mutex;

use crate::openhuman::life_capture::chronicle::sessions::rules::{
    classify, extend, BoundaryReason, EventView, OpenSession,
};
use crate::openhuman::life_capture::chronicle::sessions::tables::{
    insert_closed_session, roll_up_minute_buckets, ClosedSession,
};
use crate::openhuman::life_capture::chronicle::tables::{get_watermark, set_watermark};
use crate::openhuman::life_capture::index::PersonalIndex;

/// Watermark source name — must not collide with other chronicle
/// watermarks. Persisted in chronicle_watermark(source).
pub const WATERMARK_SOURCE: &str = "session_manager";

/// One pass's effect: how many events were consumed and how many sessions
/// closed during this tick. Handy for test assertions and logs.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TickResult {
    pub events_consumed: usize,
    pub sessions_closed: usize,
}

/// Ordered event buffer with the chronicle_events row fields the manager
/// needs. Kept private so we don't leak the row shape to callers.
#[derive(Debug, Clone)]
struct RowView {
    ts_ms: i64,
    focused_app: String,
}

/// Manager state — the currently-open session plus an in-progress event
/// log for minute-bucket rollup. Wrapped in Arc<Mutex<_>> by the runtime
/// so the ticker can be shared.
#[derive(Debug, Default)]
pub struct SessionManager {
    open: Option<OpenSession>,
    /// Raw events of the open session, kept as (ts_ms, app) so we can
    /// compute minute buckets when we close. Cleared on close.
    pending_events: Vec<(i64, String)>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Visible for tests: is a session currently open?
    pub fn has_open(&self) -> bool {
        self.open.is_some()
    }

    /// Run one tick: read new chronicle_events past the watermark, apply
    /// boundary rules, persist any closed sessions, advance the
    /// watermark. Safe to call concurrently-but-not-interleaved — the
    /// caller (runtime) serialises ticks.
    pub async fn tick(&mut self, idx: &PersonalIndex) -> anyhow::Result<TickResult> {
        let cursor = get_watermark(idx, WATERMARK_SOURCE.into())
            .await?
            .unwrap_or(i64::MIN);
        let rows = fetch_events_after(idx, cursor).await?;

        let mut closed_count = 0usize;
        let mut last_folded_ts: Option<i64> = None;

        for row in &rows {
            let view = EventView {
                ts_ms: row.ts_ms,
                focused_app: row.focused_app.clone(),
            };

            // No open session → start one and continue to the next event.
            let Some(open) = self.open.as_mut() else {
                self.open = Some(OpenSession::start(&view));
                self.pending_events.push((view.ts_ms, view.focused_app));
                continue;
            };

            // Open session exists — check for a boundary.
            match classify(open, &view) {
                Some(reason) => {
                    // Close the current session using the last event we'd
                    // already folded in (NOT this breaking event).
                    let closed = self.build_closed(reason);
                    let close_ts_ms = closed.close_ts_ms;
                    insert_closed_session(idx, closed).await?;
                    closed_count += 1;
                    last_folded_ts = Some(close_ts_ms);

                    // Start the next session from this event.
                    self.open = Some(OpenSession::start(&view));
                    self.pending_events.push((view.ts_ms, view.focused_app));
                }
                None => {
                    extend(open, &view);
                    self.pending_events.push((view.ts_ms, view.focused_app));
                }
            }
        }

        // Advance watermark only up to the last ts_ms we folded into a
        // closed session. Events inside the still-open session are NOT
        // watermarked yet — a restart replays them and re-opens the same
        // session. This keeps the "crash-safe, no duplicate sessions"
        // invariant: closed sessions correspond 1:1 to event spans.
        if let Some(ts) = last_folded_ts {
            set_watermark(idx, WATERMARK_SOURCE.into(), ts).await?;
        }

        Ok(TickResult {
            events_consumed: rows.len(),
            sessions_closed: closed_count,
        })
    }

    fn build_closed(&mut self, reason: BoundaryReason) -> ClosedSession {
        let open = self
            .open
            .take()
            .expect("build_closed called without an open session");
        let events = std::mem::take(&mut self.pending_events);
        let buckets = roll_up_minute_buckets(&events);
        ClosedSession {
            start_ts_ms: open.start_ts_ms,
            close_ts_ms: open.last_ts_ms,
            boundary_reason: reason,
            primary_app: open.primary_app,
            event_count: events.len() as i64,
            minute_buckets: buckets,
        }
    }
}

/// Read all chronicle_events with ts_ms strictly greater than `cursor`,
/// newest-last so the manager folds them in chronological order.
async fn fetch_events_after(idx: &PersonalIndex, cursor: i64) -> anyhow::Result<Vec<RowView>> {
    let conn: Arc<Mutex<rusqlite::Connection>> = idx.writer.clone();
    tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<RowView>> {
        let guard = conn.blocking_lock();
        let mut stmt = guard
            .prepare(
                "SELECT ts_ms, focused_app FROM chronicle_events \
                 WHERE ts_ms > ?1 \
                 ORDER BY ts_ms ASC, id ASC",
            )
            .context("prepare fetch_events_after")?;
        let rows = stmt
            .query_map(params![cursor], |r| {
                Ok(RowView {
                    ts_ms: r.get(0)?,
                    focused_app: r.get(1)?,
                })
            })
            .context("query fetch_events_after")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("collect fetch_events_after rows")?;
        Ok(rows)
    })
    .await
    .context("fetch_events_after task panicked")?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::life_capture::chronicle::parser::ChronicleEvent;
    use crate::openhuman::life_capture::chronicle::sessions::rules::{
        APP_SWITCH_MS, IDLE_GAP_MS, MAX_SESSION_MS,
    };
    use crate::openhuman::life_capture::chronicle::sessions::tables::count_sessions;
    use crate::openhuman::life_capture::chronicle::tables::insert_event;

    fn raw(app: &str, ts: i64) -> ChronicleEvent {
        ChronicleEvent {
            focused_app: app.into(),
            focused_element: None,
            visible_text: None,
            url: None,
            ts_ms: ts,
        }
    }

    async fn seed(idx: &PersonalIndex, events: &[(&str, i64)]) {
        for (app, ts) in events {
            insert_event(idx, raw(app, *ts)).await.unwrap();
        }
    }

    #[tokio::test]
    async fn idle_gap_closes_session_and_starts_a_new_one() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        seed(
            &idx,
            &[
                ("code", 0),
                ("code", 60_000),
                // 6-minute gap.
                ("code", 60_000 + IDLE_GAP_MS + 60_000),
                ("code", 60_000 + IDLE_GAP_MS + 120_000),
            ],
        )
        .await;
        let mut mgr = SessionManager::new();
        let r = mgr.tick(&idx).await.unwrap();
        assert_eq!(r.events_consumed, 4);
        assert_eq!(
            r.sessions_closed, 1,
            "idle gap must close exactly one session"
        );
        assert_eq!(count_sessions(&idx).await.unwrap(), 1);
        assert!(mgr.has_open(), "second session remains open");
    }

    #[tokio::test]
    async fn app_switch_stretch_closes_session() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        seed(
            &idx,
            &[
                ("code", 0),
                ("slack", 60_000),
                // Continuous slack for 3m + 1s from switch start → switch fires.
                ("slack", 60_000 + APP_SWITCH_MS + 1_000),
            ],
        )
        .await;
        let mut mgr = SessionManager::new();
        let r = mgr.tick(&idx).await.unwrap();
        assert_eq!(r.sessions_closed, 1);
    }

    #[tokio::test]
    async fn max_duration_forces_split_under_constant_activity() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        // Feed one event per minute for 2h 1min, all on the same app.
        let mut seeded: Vec<(&str, i64)> = Vec::new();
        let mut t = 0i64;
        while t <= MAX_SESSION_MS {
            seeded.push(("code", t));
            t += 60_000;
        }
        seed(&idx, &seeded).await;
        let mut mgr = SessionManager::new();
        let r = mgr.tick(&idx).await.unwrap();
        assert_eq!(r.sessions_closed, 1, "exactly one split at the 2h mark");
    }

    #[tokio::test]
    async fn watermark_advances_past_closed_sessions_only() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        seed(
            &idx,
            &[
                ("code", 0),
                ("code", 60_000),
                ("code", IDLE_GAP_MS + 60_000), // 6m gap → break
                ("code", IDLE_GAP_MS + 120_000),
            ],
        )
        .await;
        let mut mgr = SessionManager::new();
        mgr.tick(&idx).await.unwrap();
        let cursor = get_watermark(&idx, WATERMARK_SOURCE.into())
            .await
            .unwrap()
            .expect("watermark set");
        // First session's last event was ts=60_000 → watermark advances to that.
        // The events inside the open second session are not yet committed.
        assert_eq!(cursor, 60_000);
    }

    #[tokio::test]
    async fn restart_resumes_from_watermark_without_duplicating_sessions() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        seed(
            &idx,
            &[
                ("code", 0),
                ("code", 60_000),
                ("code", IDLE_GAP_MS + 60_000),
                ("code", IDLE_GAP_MS + 120_000),
            ],
        )
        .await;

        // First process: closes first session, second stays open.
        let mut mgr1 = SessionManager::new();
        mgr1.tick(&idx).await.unwrap();
        assert_eq!(count_sessions(&idx).await.unwrap(), 1);

        // Simulate restart — fresh manager, same DB. Feed another gap to
        // force the still-open session to close.
        seed(
            &idx,
            &[("code", IDLE_GAP_MS + 120_000 + IDLE_GAP_MS + 60_000)],
        )
        .await;
        let mut mgr2 = SessionManager::new();
        mgr2.tick(&idx).await.unwrap();
        // Should now have exactly 2 sessions total — no duplicates from
        // the resumed events.
        assert_eq!(count_sessions(&idx).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn empty_tick_is_a_noop() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        let mut mgr = SessionManager::new();
        let r = mgr.tick(&idx).await.unwrap();
        assert_eq!(r, TickResult::default());
        assert!(!mgr.has_open());
    }
}
