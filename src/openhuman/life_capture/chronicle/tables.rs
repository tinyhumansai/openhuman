//! SQLite access layer for chronicle_events + chronicle_watermark.
//!
//! Mirrors the life_capture writer/reader split minus the FTS + vec
//! machinery: chronicle rows are structured text, not semantic payload, and
//! A3 doesn't need fuzzy search over them. Callers hand in a
//! `&PersonalIndex` so chronicle shares the single on-disk db with the rest
//! of life_capture — migrations have already created the tables.

use anyhow::Context;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;

use crate::openhuman::life_capture::chronicle::parser::ChronicleEvent;
use crate::openhuman::life_capture::index::PersonalIndex;

/// Row as returned to RPC callers. Kept flat so the controller schema shape
/// matches without a remapping pass.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StoredEvent {
    pub id: i64,
    pub ts_ms: i64,
    pub focused_app: String,
    pub focused_element: Option<String>,
    pub visible_text: Option<String>,
    pub url: Option<String>,
}

/// Insert a parsed chronicle event. Returns the autoincremented row id.
pub async fn insert_event(idx: &PersonalIndex, event: ChronicleEvent) -> anyhow::Result<i64> {
    let conn = idx.writer.clone();
    tokio::task::spawn_blocking(move || -> anyhow::Result<i64> {
        let guard = conn.blocking_lock();
        guard
            .execute(
                "INSERT INTO chronicle_events(ts_ms, focused_app, focused_element, visible_text, url) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    event.ts_ms,
                    event.focused_app,
                    event.focused_element,
                    event.visible_text,
                    event.url,
                ],
            )
            .context("insert chronicle_events row")?;
        Ok(guard.last_insert_rowid())
    })
    .await
    .context("insert_event task panicked")?
}

/// Most recent events, newest first, capped at `limit`.
pub async fn list_recent(idx: &PersonalIndex, limit: i64) -> anyhow::Result<Vec<StoredEvent>> {
    let conn = idx.writer.clone();
    tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<StoredEvent>> {
        let guard = conn.blocking_lock();
        let mut stmt = guard
            .prepare(
                "SELECT id, ts_ms, focused_app, focused_element, visible_text, url \
                 FROM chronicle_events \
                 ORDER BY ts_ms DESC, id DESC \
                 LIMIT ?1",
            )
            .context("prepare list_recent")?;
        let rows = stmt
            .query_map(params![limit], |row| {
                Ok(StoredEvent {
                    id: row.get(0)?,
                    ts_ms: row.get(1)?,
                    focused_app: row.get(2)?,
                    focused_element: row.get(3)?,
                    visible_text: row.get(4)?,
                    url: row.get(5)?,
                })
            })
            .context("query list_recent")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("collect list_recent rows")?;
        Ok(rows)
    })
    .await
    .context("list_recent task panicked")?
}

pub async fn get_watermark(idx: &PersonalIndex, source: String) -> anyhow::Result<Option<i64>> {
    let conn = idx.writer.clone();
    tokio::task::spawn_blocking(move || -> anyhow::Result<Option<i64>> {
        let guard = conn.blocking_lock();
        let ts: Option<i64> = guard
            .query_row(
                "SELECT last_ts_ms FROM chronicle_watermark WHERE source = ?1",
                params![source],
                |row| row.get(0),
            )
            .optional()
            .context("select chronicle_watermark")?;
        Ok(ts)
    })
    .await
    .context("get_watermark task panicked")?
}

pub async fn set_watermark(idx: &PersonalIndex, source: String, ts_ms: i64) -> anyhow::Result<()> {
    let conn = idx.writer.clone();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let guard = conn.blocking_lock();
        guard
            .execute(
                // MAX() guards against out-of-order / retry writes moving the
                // cursor backward, which would replay already-processed rows.
                "INSERT INTO chronicle_watermark(source, last_ts_ms) VALUES (?1, ?2) \
                 ON CONFLICT(source) DO UPDATE SET \
                   last_ts_ms = MAX(chronicle_watermark.last_ts_ms, excluded.last_ts_ms)",
                params![source, ts_ms],
            )
            .context("upsert chronicle_watermark")?;
        Ok(())
    })
    .await
    .context("set_watermark task panicked")?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(app: &str, ts: i64) -> ChronicleEvent {
        ChronicleEvent {
            focused_app: app.into(),
            focused_element: None,
            visible_text: None,
            url: None,
            ts_ms: ts,
        }
    }

    #[tokio::test]
    async fn insert_and_list_recent_returns_newest_first() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        insert_event(&idx, event("a", 100)).await.unwrap();
        insert_event(&idx, event("b", 300)).await.unwrap();
        insert_event(&idx, event("c", 200)).await.unwrap();

        let rows = list_recent(&idx, 10).await.unwrap();
        let order: Vec<_> = rows.iter().map(|r| r.focused_app.clone()).collect();
        assert_eq!(order, vec!["b", "c", "a"]);
    }

    #[tokio::test]
    async fn list_recent_respects_limit() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        for i in 0..5 {
            insert_event(&idx, event("x", i)).await.unwrap();
        }
        let rows = list_recent(&idx, 2).await.unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[tokio::test]
    async fn watermark_round_trip() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        assert_eq!(
            get_watermark(&idx, "focus".into()).await.unwrap(),
            None,
            "fresh source has no watermark"
        );
        set_watermark(&idx, "focus".into(), 12345).await.unwrap();
        assert_eq!(
            get_watermark(&idx, "focus".into()).await.unwrap(),
            Some(12345)
        );
        // Overwrite.
        set_watermark(&idx, "focus".into(), 67890).await.unwrap();
        assert_eq!(
            get_watermark(&idx, "focus".into()).await.unwrap(),
            Some(67890)
        );
    }

    #[tokio::test]
    async fn watermark_never_moves_backward() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        set_watermark(&idx, "focus".into(), 100).await.unwrap();
        // Stale / out-of-order write should be ignored.
        set_watermark(&idx, "focus".into(), 50).await.unwrap();
        assert_eq!(
            get_watermark(&idx, "focus".into()).await.unwrap(),
            Some(100),
            "watermark must not regress below its previous value"
        );
        // Equal write is a no-op.
        set_watermark(&idx, "focus".into(), 100).await.unwrap();
        assert_eq!(
            get_watermark(&idx, "focus".into()).await.unwrap(),
            Some(100)
        );
        // Forward progress still works.
        set_watermark(&idx, "focus".into(), 200).await.unwrap();
        assert_eq!(
            get_watermark(&idx, "focus".into()).await.unwrap(),
            Some(200)
        );
    }

    #[tokio::test]
    async fn watermarks_are_keyed_per_source() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        set_watermark(&idx, "focus".into(), 1).await.unwrap();
        set_watermark(&idx, "calendar".into(), 2).await.unwrap();
        assert_eq!(get_watermark(&idx, "focus".into()).await.unwrap(), Some(1));
        assert_eq!(
            get_watermark(&idx, "calendar".into()).await.unwrap(),
            Some(2)
        );
    }
}
