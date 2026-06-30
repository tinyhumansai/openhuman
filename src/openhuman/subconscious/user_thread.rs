//! The user-facing thread + the `notify_user` handoff tool.
//!
//! The background orchestrator's reasoning lives in its own reserved thread
//! (`subconscious:orchestrator`). When it decides to actually *say something*
//! to the user, it must not write to that internal thread or emit to a
//! channel directly — it hands off through this separate, long-lived
//! user-facing thread (`subconscious:user`), which is where agent↔user
//! communication is recorded.
//!
//! The handoff is the [`NotifyUserTool`]: it persists the message to the
//! user-facing thread and publishes [`DomainEvent::ProactiveMessageRequested`],
//! which the channels domain already routes to the active channel. Keeping
//! delivery on the existing proactive path (rather than synthesizing an
//! inbound message) means the orchestrator can never trigger itself via its
//! own output.

use async_trait::async_trait;
use serde_json::json;
use tracing::{info, warn};

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::memory_conversations::ConversationMessage;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCategory, ToolResult, ToolScope};

/// Reserved conversation thread for agent↔user communication, distinct from
/// the orchestrator's internal reasoning thread.
pub const USER_THREAD_ID: &str = "subconscious:user";

/// Source tag on proactive deliveries originating from the subconscious.
/// Mirrors [`crate::openhuman::subconscious_triggers::SUBCONSCIOUS_SENDER_MARKER`]
/// so the trigger fan-in can recognise (and skip) the orchestrator's own
/// output.
pub const SUBCONSCIOUS_PROACTIVE_SOURCE: &str = "subconscious";

/// Persist a message to the user-facing thread and request its proactive
/// delivery. Best-effort persistence: a storage failure is logged but does
/// not prevent delivery.
pub fn notify_user(workspace_dir: std::path::PathBuf, message: &str, subject: Option<&str>) {
    let record = ConversationMessage {
        id: uuid::Uuid::new_v4().to_string(),
        content: message.to_string(),
        message_type: "text".to_string(),
        extra_metadata: json!({ "origin": "subconscious_notify_user" }),
        sender: "agent".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    // `append_message` requires the thread to exist; create the reserved
    // user-facing thread lazily (idempotent).
    super::session::ensure_reserved_thread(&workspace_dir, USER_THREAD_ID, "Subconscious → You");
    if let Err(err) = crate::openhuman::memory_conversations::append_message(
        workspace_dir,
        USER_THREAD_ID,
        record,
    ) {
        warn!("[subconscious::user_thread] persist notify_user message failed: {err}");
    }

    publish_global(DomainEvent::ProactiveMessageRequested {
        source: SUBCONSCIOUS_PROACTIVE_SOURCE.to_string(),
        message: message.to_string(),
        job_name: subject.map(|s| s.to_string()),
    });
    info!(
        "[subconscious::user_thread] notify_user delivered ({} chars)",
        message.chars().count()
    );
}

/// Agent tool: hand a message off to the user via the user-facing thread.
pub struct NotifyUserTool;

#[async_trait]
impl Tool for NotifyUserTool {
    fn name(&self) -> &str {
        "notify_user"
    }

    fn description(&self) -> &str {
        "Proactively send a message to the user. Use this when the background \
         loop has something worth surfacing — a finding, a reminder, a heads-up. \
         The message is delivered to the user's active channel and recorded in \
         the user-facing conversation thread. Keep it concise and high-signal; \
         do not narrate routine background bookkeeping."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["message"],
            "properties": {
                "message": {
                    "type": "string",
                    "description": "The message to send to the user."
                },
                "subject": {
                    "type": "string",
                    "description": "Optional short subject/label for threading and display."
                }
            }
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    fn permission_level(&self) -> PermissionLevel {
        // Outward-facing: surfaces content to the user. Treated as a write
        // so policy/approval can gate it under stricter autonomy tiers.
        PermissionLevel::Write
    }

    fn scope(&self) -> ToolScope {
        ToolScope::AgentOnly
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("notify_user: `message` is required and non-empty"))?;
        let subject = args.get("subject").and_then(|v| v.as_str());

        let config = crate::openhuman::config::load_config_with_timeout()
            .await
            .map_err(|e| anyhow::anyhow!("config load: {e}"))?;

        notify_user(config.workspace_dir, message, subject);
        Ok(ToolResult::success(
            "Message delivered to the user.".to_string(),
        ))
    }
}

/// All user-facing-thread tools, for registration into the tool registry.
pub fn all_user_thread_tools() -> Vec<Box<dyn Tool>> {
    vec![Box::new(NotifyUserTool)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_thread_id_is_distinct_from_orchestrator() {
        assert_eq!(USER_THREAD_ID, "subconscious:user");
        assert_ne!(USER_THREAD_ID, super::super::ORCHESTRATOR_THREAD_ID);
    }

    #[test]
    fn notify_user_tool_metadata() {
        let tool = NotifyUserTool;
        assert_eq!(tool.name(), "notify_user");
        assert_eq!(tool.scope(), ToolScope::AgentOnly);
        assert_eq!(tool.permission_level(), PermissionLevel::Write);
        let schema = tool.parameters_schema();
        assert_eq!(schema["required"][0], "message");
    }

    #[tokio::test]
    async fn notify_user_rejects_empty_message() {
        let tool = NotifyUserTool;
        let err = tool.execute(json!({ "message": "   " })).await.unwrap_err();
        assert!(err.to_string().contains("message"));
    }

    #[test]
    fn proactive_source_matches_trigger_marker() {
        assert_eq!(
            SUBCONSCIOUS_PROACTIVE_SOURCE,
            crate::openhuman::subconscious_triggers::SUBCONSCIOUS_SENDER_MARKER
        );
    }
}
