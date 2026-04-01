//! Tool: insert_sql_record — insert an episodic record into the FTS5 memory database.

use super::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

/// Valid values for the `role` parameter.
const VALID_ROLES: &[&str] = &["user", "assistant", "tool"];

/// Inserts an episodic memory record into the FTS5 episodic-memory SQLite table.
///
/// # Current status
/// The FTS5 schema and connection pool will be wired in Phase 5 of the harness
/// implementation. This stub validates parameters, emits structured trace logs,
/// and returns a success result so calling agents can proceed without blocking.
pub struct InsertSqlRecordTool;

impl InsertSqlRecordTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for InsertSqlRecordTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for InsertSqlRecordTool {
    fn name(&self) -> &str {
        "insert_sql_record"
    }

    fn description(&self) -> &str {
        "Insert an episodic memory record into the FTS5 memory database. \
         Records are tagged with a session ID, role (user/assistant/tool), \
         content, and an optional extracted lesson. The database enables \
         full-text search over conversation history for future retrieval."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["session_id", "role", "content"],
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "Unique identifier for the conversation session."
                },
                "role": {
                    "type": "string",
                    "enum": ["user", "assistant", "tool"],
                    "description": "Who produced this record."
                },
                "content": {
                    "type": "string",
                    "description": "The text content of the message or tool output."
                },
                "lesson": {
                    "type": "string",
                    "description": "Optional distilled lesson extracted from this exchange."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        // ── Parameter extraction ────────────────────────────────────────────
        let session_id = args
            .get("session_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter 'session_id'"))?;

        let role = args
            .get("role")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter 'role'"))?;

        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter 'content'"))?;

        let lesson = args.get("lesson").and_then(|v| v.as_str());

        // ── Validation ──────────────────────────────────────────────────────
        if !VALID_ROLES.contains(&role) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Invalid role '{role}'. Must be one of: user, assistant, tool."
                )),
            });
        }

        if session_id.trim().is_empty() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("'session_id' must not be empty.".into()),
            });
        }

        if content.trim().is_empty() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("'content' must not be empty.".into()),
            });
        }

        // ── Structured trace log ────────────────────────────────────────────
        tracing::info!(
            session_id = session_id,
            role = role,
            content_len = content.len(),
            has_lesson = lesson.is_some(),
            "[insert_sql_record] episodic record queued for FTS5 insert"
        );

        if let Some(lesson_text) = lesson {
            tracing::debug!(
                session_id = session_id,
                "[insert_sql_record] lesson: {lesson_text}"
            );
        }

        // ── Placeholder result (FTS5 wire-up deferred to Phase 5) ───────────
        // TODO(phase-5): obtain `Arc<SqlitePool>` from app state, run:
        //   sqlx::query!(
        //       "INSERT INTO episodic_memory(session_id, role, content, lesson, ts)
        //        VALUES (?, ?, ?, ?, unixepoch())",
        //       session_id, role, content, lesson
        //   ).execute(&*pool).await?;
        let summary = format!(
            "Record staged: session={session_id} role={role} content_len={} lesson={}",
            content.len(),
            lesson.map_or("none".to_string(), |l| format!("{} chars", l.len())),
        );

        Ok(ToolResult {
            success: true,
            output: summary,
            error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> InsertSqlRecordTool {
        InsertSqlRecordTool::new()
    }

    #[tokio::test]
    async fn inserts_minimal_record() {
        let result = tool()
            .execute(json!({
                "session_id": "sess-001",
                "role": "user",
                "content": "Hello, world!"
            }))
            .await
            .unwrap();
        assert!(result.success, "{:?}", result.error);
        assert!(result.output.contains("sess-001"));
        assert!(result.output.contains("user"));
    }

    #[tokio::test]
    async fn inserts_with_lesson() {
        let result = tool()
            .execute(json!({
                "session_id": "sess-002",
                "role": "assistant",
                "content": "Use cargo fmt before committing.",
                "lesson": "Always format Rust code before review."
            }))
            .await
            .unwrap();
        assert!(result.success, "{:?}", result.error);
        assert!(result.output.contains("lesson="));
    }

    #[tokio::test]
    async fn rejects_invalid_role() {
        let result = tool()
            .execute(json!({
                "session_id": "sess-003",
                "role": "system",
                "content": "Invalid role test."
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result
            .error
            .as_deref()
            .unwrap_or("")
            .contains("Invalid role"));
    }

    #[tokio::test]
    async fn rejects_empty_session_id() {
        let result = tool()
            .execute(json!({
                "session_id": "  ",
                "role": "user",
                "content": "Some content."
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap_or("").contains("session_id"));
    }

    #[tokio::test]
    async fn rejects_empty_content() {
        let result = tool()
            .execute(json!({
                "session_id": "sess-004",
                "role": "tool",
                "content": ""
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap_or("").contains("content"));
    }

    #[tokio::test]
    async fn missing_required_param_returns_error() {
        let result = tool()
            .execute(json!({ "session_id": "s", "role": "user" }))
            .await;
        assert!(result.is_err(), "should return Err for missing 'content'");
    }

    #[test]
    fn schema_has_required_fields() {
        let schema = tool().parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("session_id")));
        assert!(required.contains(&json!("role")));
        assert!(required.contains(&json!("content")));
    }

    #[test]
    fn permission_is_write() {
        assert_eq!(tool().permission_level(), PermissionLevel::Write);
    }
}
