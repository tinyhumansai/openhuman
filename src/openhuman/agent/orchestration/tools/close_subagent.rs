//! Tool: `close_subagent` - retire a reusable durable sub-agent session.

use crate::openhuman::agent::harness::fork_context::current_parent;
use crate::openhuman::agent::orchestration::subagent_sessions::SubagentSessionStore;
use crate::openhuman::agent::orchestration::{running_subagents, subagent_sessions};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

pub struct CloseSubagentTool;

impl CloseSubagentTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CloseSubagentTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for CloseSubagentTool {
    fn name(&self) -> &str {
        "close_subagent"
    }

    fn description(&self) -> &str {
        "Close a reusable sub-agent session so future delegation creates a fresh worker. \
         If that session is currently running, it is cancelled first."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["subagent_session_id"],
            "properties": {
                "subagent_session_id": {
                    "type": "string",
                    "description": "Durable subagent_session_id returned by reusable async delegation."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let subagent_session_id = args
            .get("subagent_session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if subagent_session_id.is_empty() {
            return Ok(ToolResult::error(
                "close_subagent: `subagent_session_id` is required",
            ));
        }
        let parent = match current_parent() {
            Some(parent) => parent,
            None => {
                return Ok(ToolResult::error(
                    "close_subagent called outside of an agent turn",
                ));
            }
        };
        let store = SubagentSessionStore::new(parent.workspace_dir.clone());
        let parent_thread_id =
            crate::openhuman::agent::tinyagents::thread_context::current_thread_id();
        let owned = match subagent_sessions::list_for_parent(
            &store,
            &parent.session_id,
            parent_thread_id.as_deref(),
        ) {
            Ok(sessions) => sessions
                .iter()
                .any(|session| session.subagent_session_id == subagent_session_id),
            Err(err) => {
                return Ok(ToolResult::error(format!(
                    "close_subagent: failed to read sub-agent sessions: {err}"
                )));
            }
        };
        if !owned {
            log::warn!(
                "[subagent_reuse] close rejected parent_session={} parent_thread_id={} subagent_session_id={}",
                parent.session_id,
                parent_thread_id.as_deref().unwrap_or("none"),
                subagent_session_id
            );
            return Ok(ToolResult::error(
                "close_subagent: sub-agent session not found for this parent thread",
            ));
        }
        let cancelled = running_subagents::cancel_by_session_in_workspace(
            &subagent_session_id,
            &parent.session_id,
            &parent.workspace_dir,
        )
        .is_some();
        match subagent_sessions::close(&store, &subagent_session_id) {
            Ok(closed) => {
                log::info!(
                    "[subagent_reuse] close subagent_session_id={} parent_session={} closed={} cancelled_running={}",
                    subagent_session_id,
                    parent.session_id,
                    closed,
                    cancelled
                );
                Ok(ToolResult::success(format!(
                    "Closed reusable sub-agent session `{subagent_session_id}` (closed={closed}, cancelled_running={cancelled})."
                )))
            }
            Err(err) => Ok(ToolResult::error(format!(
                "close_subagent: failed to update sub-agent session: {err}"
            ))),
        }
    }
}

#[cfg(test)]
#[path = "close_subagent_tests.rs"]
mod tests;
