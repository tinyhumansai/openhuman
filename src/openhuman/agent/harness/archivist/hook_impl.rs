//! `PostTurnHook` implementation for `ArchivistHook`.

use super::helpers::extract_lesson_from_tools;
use super::types::ArchivistHook;
use crate::openhuman::agent::hooks::{PostTurnHook, TurnContext};
use crate::openhuman::memory_store::fts5::{self, EpisodicEntry};
use async_trait::async_trait;

#[async_trait]
impl PostTurnHook for ArchivistHook {
    fn name(&self) -> &str {
        "archivist"
    }

    async fn on_turn_complete(&self, ctx: &TurnContext) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }

        let Some(conn) = &self.conn else {
            return Ok(());
        };

        let session_id = ctx.session_id.as_deref().unwrap_or("unknown");
        let timestamp = Self::now_timestamp();

        tracing::debug!(
            "[archivist] indexing turn: session={session_id}, tools={}, duration={}ms",
            ctx.tool_calls.len(),
            ctx.turn_duration_ms
        );

        // Index user message.
        fts5::episodic_insert(
            conn,
            &EpisodicEntry {
                id: None,
                session_id: session_id.to_string(),
                timestamp,
                role: "user".to_string(),
                content: ctx.user_message.clone(),
                lesson: None,
                tool_calls_json: None,
                cost_microdollars: 0,
            },
        )?;

        // Retrieve the inserted episodic ID for segment tracking.
        let current_episodic_id = {
            let db = conn.lock();
            db.query_row("SELECT last_insert_rowid()", [], |row| row.get::<_, i64>(0))
                .unwrap_or(1)
        };

        // Index assistant response with tool call summary.
        let tool_calls_json = if ctx.tool_calls.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&ctx.tool_calls).unwrap_or_default())
        };

        // Extract a simple lesson from tool failures (lightweight, no LLM needed).
        let lesson = extract_lesson_from_tools(&ctx.tool_calls);

        fts5::episodic_insert(
            conn,
            &EpisodicEntry {
                id: None,
                session_id: session_id.to_string(),
                // Offset by 1ms so assistant entries sort after user entries within
                // the same turn. Relies on turn timestamps having >=1ms resolution.
                timestamp: timestamp + 0.001,
                role: "assistant".to_string(),
                content: ctx.assistant_response.clone(),
                lesson,
                tool_calls_json,
                cost_microdollars: 0,
            },
        )?;

        tracing::debug!("[archivist] episodic rows written: session={session_id}");

        // Dual-write into memory_archivist::store (md-backed) so we can
        // validate the FTS5 → md migration before flipping the read side.
        // Best-effort: a write failure here must not break the turn. The
        // user turn's assigned seq is captured into `current_seq` so the
        // segment ops can store it alongside the FTS5 episodic id.
        let mut current_seq: Option<u32> = None;
        if let Some(cfg) = self.config.as_ref() {
            let ts_ms = (timestamp * 1000.0) as i64;
            let user_turn = crate::openhuman::memory_archivist::ArchivedTurn {
                session_id: session_id.to_string(),
                seq: 0, // assigned by record_turn
                timestamp_ms: ts_ms,
                role: "user".to_string(),
                content: ctx.user_message.clone(),
                lesson: None,
                tool_calls_json: None,
                cost_microdollars: 0,
            };
            match crate::openhuman::memory_archivist::store::record_turn(cfg, user_turn) {
                Ok(stored) => current_seq = Some(stored.seq),
                Err(e) => {
                    tracing::warn!("[archivist] memory_archivist user dual-write failed: {e}");
                }
            }
            // Assistant turn carries the tool_calls_json + lesson the FTS5
            // insert just wrote. Re-derive locally so we don't depend on
            // FTS5 having returned.
            let assistant_lesson = extract_lesson_from_tools(&ctx.tool_calls);
            let assistant_tool_calls = if ctx.tool_calls.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&ctx.tool_calls).unwrap_or_default())
            };
            let assistant_turn = crate::openhuman::memory_archivist::ArchivedTurn {
                session_id: session_id.to_string(),
                seq: 0,
                timestamp_ms: ts_ms + 1,
                role: "assistant".to_string(),
                content: ctx.assistant_response.clone(),
                lesson: assistant_lesson,
                tool_calls_json: assistant_tool_calls,
                cost_microdollars: 0,
            };
            if let Err(e) =
                crate::openhuman::memory_archivist::store::record_turn(cfg, assistant_turn)
            {
                tracing::warn!("[archivist] memory_archivist assistant dual-write failed: {e}");
            }
        }

        // Manage conversation segmentation (sync boundary detection + SQLite
        // operations). Returns the just-closed segment when a boundary fired.
        let closed_segment = self.manage_segment_sync(
            conn,
            session_id,
            timestamp,
            &ctx.user_message,
            current_episodic_id,
            current_seq,
        );

        // Run async recap + embed + segment-tree ingest on the closed segment
        // (if any). Per-turn tree ingest is intentionally absent — Phase 2
        // moves the tree write to segment granularity inside on_segment_closed.
        if let Some(ref segment) = closed_segment {
            let now = Self::now_timestamp();
            self.on_segment_closed(conn, segment, session_id, now).await;
        }

        tracing::debug!("[archivist] turn indexed successfully: session={session_id}");
        Ok(())
    }
}
