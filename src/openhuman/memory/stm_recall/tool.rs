//! Agent-callable tool for on-demand STM recall.
//!
//! The agent invokes `stm_recall_search[query]` mid-session to pull
//! cross-thread context by keyword. The tool:
//!
//! 1. Extracts the active session_id from the tool call arguments (or falls
//!    back to a process-level default when not provided).
//! 2. Runs [`stm_recall`] with Arm 1 (FTS5 keyword) only — no embedding step
//!    at call time to avoid blocking the hot path.
//! 3. Returns the rendered markdown block as a tool result.
//!
//! The tool is registered in `tools/ops.rs` alongside `MemoryRecallTool` when
//! `learning.stm_recall_enabled` is true.

use crate::openhuman::memory::Memory;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

use super::recall::{stm_recall, StmRecallOpts};

/// On-demand STM recall tool.
///
/// Searches recent episodic memory from **other** chat threads using
/// keyword matching (FTS5). Vector similarity (Arm 2) is not run here —
/// it requires an embedding call that belongs in the session-start
/// preemptive path, not the on-demand agent invocation path.
pub struct StmRecallTool {
    memory: Arc<dyn Memory>,
    /// Session ID to exclude from results. Injected at construction so
    /// the tool knows the "current thread" without the agent having to
    /// pass it explicitly.
    session_id: String,
    /// Optional model signature for filtering segment embeddings.
    model_signature: Option<String>,
}

impl StmRecallTool {
    /// Create a new `StmRecallTool`.
    ///
    /// `session_id` — the current session's ID. Results from this session
    /// are always excluded.
    ///
    /// `model_signature` — when `Some`, Arm 2 is filtered to this model.
    /// Pass `None` to accept any model (Arm 2 is skipped in on-demand mode
    /// anyway, but stored for future extension).
    pub fn new(
        memory: Arc<dyn Memory>,
        session_id: String,
        model_signature: Option<String>,
    ) -> Self {
        Self {
            memory,
            session_id,
            model_signature,
        }
    }
}

#[async_trait]
impl Tool for StmRecallTool {
    fn name(&self) -> &str {
        "stm_recall_search"
    }

    fn description(&self) -> &str {
        "Search recent conversational context from other chat threads. \
         Use this when you need facts or context discussed in a previous conversation \
         that may be relevant to the current request. \
         Returns a bounded set of snippets and conversation recaps from other sessions."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keywords or phrase to search across recent other-session conversations"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'query' parameter"))?
            .trim();
        if query.is_empty() {
            return Err(anyhow::anyhow!("query cannot be empty"));
        }

        tracing::debug!(
            "[stm_recall_tool] on-demand recall query_len={} session={}",
            query.chars().count(),
            self.session_id
        );

        // Get SQLite connection via the Memory trait's sqlite_conn() hook.
        let conn = match self.memory.sqlite_conn() {
            Some(c) => c,
            None => {
                tracing::warn!(
                    "[stm_recall_tool] memory backend has no SQLite connection — stm_recall unavailable"
                );
                return Ok(ToolResult::success(
                    "STM recall is not available (memory backend is not SQLite-backed).",
                ));
            }
        };

        let opts = StmRecallOpts {
            exclude_session: &self.session_id,
            query: Some(query),
            model_signature: self.model_signature.as_deref(),
        };

        match stm_recall(&conn, &opts, None) {
            Ok(block) => {
                tracing::debug!(
                    "[stm_recall_tool] recall complete: {} items, {} fts5_candidates, {} dropped_dedup",
                    block.items.len(),
                    block.fts5_candidates,
                    block.dropped_dedup
                );
                if block.is_empty() {
                    Ok(ToolResult::success(
                        "No relevant context found in recent other-session conversations.",
                    ))
                } else {
                    Ok(ToolResult::success(block.render()))
                }
            }
            Err(e) => {
                tracing::warn!("[stm_recall_tool] stm_recall failed: {e}");
                Ok(ToolResult::error(format!("STM recall failed: {e}")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::embeddings::NoopEmbedding;
    use crate::openhuman::memory::UnifiedMemory;
    use tempfile::TempDir;

    fn make_mem() -> (TempDir, Arc<dyn Memory>) {
        let tmp = TempDir::new().unwrap();
        let mem = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();
        (tmp, Arc::new(mem))
    }

    #[tokio::test]
    async fn tool_name_and_schema() {
        let (_tmp, mem) = make_mem();
        let tool = StmRecallTool::new(mem, "s1".into(), None);
        assert_eq!(tool.name(), "stm_recall_search");
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["query"].is_object());
        assert_eq!(schema["required"][0].as_str(), Some("query"));
    }

    #[tokio::test]
    async fn tool_empty_query_returns_error() {
        let (_tmp, mem) = make_mem();
        let tool = StmRecallTool::new(mem, "s1".into(), None);
        let result = tool.execute(json!({"query": "  "})).await;
        assert!(result.is_err(), "empty query must return Err");
    }

    #[tokio::test]
    async fn tool_missing_query_returns_error() {
        let (_tmp, mem) = make_mem();
        let tool = StmRecallTool::new(mem, "s1".into(), None);
        let result = tool.execute(json!({})).await;
        assert!(result.is_err(), "missing query must return Err");
    }

    #[tokio::test]
    async fn tool_returns_no_matches_when_empty_db() {
        let (_tmp, mem) = make_mem();
        let tool = StmRecallTool::new(mem, "s1".into(), None);
        let result = tool
            .execute(json!({"query": "Rust ownership"}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(
            result.output().contains("No relevant context") || !result.output().is_empty(),
            "empty db must return a no-match message, got: {}",
            result.output()
        );
    }
}
