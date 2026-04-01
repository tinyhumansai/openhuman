//! Archivist — background PostTurnHook that extracts lessons and indexes
//! episodic records.
//!
//! After each turn, the Archivist:
//! 1. Inserts the turn into the FTS5 episodic table.
//! 2. (Future) Summarises the turn using a cheap local model.
//! 3. (Future) Extracts reusable lessons → appends to MEMORY.md.

use crate::openhuman::agent::hooks::{PostTurnHook, TurnContext};
use crate::openhuman::memory::store::fts5::{self, EpisodicEntry};
use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::Connection;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Background Archivist that indexes turns into FTS5 episodic memory.
pub struct ArchivistHook {
    /// SQLite connection shared with UnifiedMemory.
    conn: Option<Arc<Mutex<Connection>>>,
    /// Whether the archivist is enabled.
    enabled: bool,
}

impl ArchivistHook {
    /// Create an Archivist hook with a shared SQLite connection.
    pub fn new(conn: Arc<Mutex<Connection>>, enabled: bool) -> Self {
        Self {
            conn: Some(conn),
            enabled,
        }
    }

    /// Create a disabled/no-op Archivist (when FTS5 is not enabled).
    pub fn disabled() -> Self {
        Self {
            conn: None,
            enabled: false,
        }
    }

    fn now_timestamp() -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }
}

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
                timestamp: timestamp + 0.001, // slightly after user message
                role: "assistant".to_string(),
                content: ctx.assistant_response.clone(),
                lesson,
                tool_calls_json,
                cost_microdollars: 0,
            },
        )?;

        tracing::debug!("[archivist] turn indexed successfully");
        Ok(())
    }
}

/// Extract simple lessons from tool call outcomes (no LLM needed).
fn extract_lesson_from_tools(
    tool_calls: &[crate::openhuman::agent::hooks::ToolCallRecord],
) -> Option<String> {
    let failures: Vec<&str> = tool_calls
        .iter()
        .filter(|tc| !tc.success)
        .map(|tc| tc.name.as_str())
        .collect();

    if failures.is_empty() {
        return None;
    }

    Some(format!(
        "Tools that failed in this turn: {}",
        failures.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::hooks::{ToolCallRecord, TurnContext};
    use crate::openhuman::memory::store::fts5;

    fn setup_conn() -> Arc<Mutex<Connection>> {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(fts5::EPISODIC_INIT_SQL).unwrap();
        Arc::new(Mutex::new(conn))
    }

    #[tokio::test]
    async fn archivist_indexes_turn() {
        let conn = setup_conn();
        let hook = ArchivistHook::new(conn.clone(), true);

        let ctx = TurnContext {
            user_message: "What is Rust?".into(),
            assistant_response: "Rust is a systems programming language.".into(),
            tool_calls: vec![],
            turn_duration_ms: 500,
            session_id: Some("test-session".into()),
            iteration_count: 1,
        };

        hook.on_turn_complete(&ctx).await.unwrap();

        let entries = fts5::episodic_session_entries(&conn, "test-session").unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].role, "user");
        assert_eq!(entries[1].role, "assistant");
    }

    #[tokio::test]
    async fn archivist_extracts_failure_lesson() {
        let conn = setup_conn();
        let hook = ArchivistHook::new(conn.clone(), true);

        let ctx = TurnContext {
            user_message: "Run tests".into(),
            assistant_response: "Tests failed.".into(),
            tool_calls: vec![ToolCallRecord {
                name: "shell".into(),
                arguments: serde_json::json!({"command": "cargo test"}),
                success: false,
                output_summary: "shell: failed (error)".into(),
                duration_ms: 3000,
            }],
            turn_duration_ms: 3500,
            session_id: Some("test-session-2".into()),
            iteration_count: 2,
        };

        hook.on_turn_complete(&ctx).await.unwrap();

        let entries = fts5::episodic_session_entries(&conn, "test-session-2").unwrap();
        let assistant_entry = entries.iter().find(|e| e.role == "assistant").unwrap();
        assert!(assistant_entry.lesson.as_ref().unwrap().contains("shell"));
    }

    #[tokio::test]
    async fn disabled_archivist_is_noop() {
        let hook = ArchivistHook::disabled();
        let ctx = TurnContext {
            user_message: "test".into(),
            assistant_response: "test".into(),
            tool_calls: vec![],
            turn_duration_ms: 0,
            session_id: None,
            iteration_count: 0,
        };
        // Should not error even without a connection.
        hook.on_turn_complete(&ctx).await.unwrap();
    }
}
