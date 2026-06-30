//! Tool: `list_subagents` - inspect reusable sub-agent sessions for this parent.

use crate::openhuman::agent::harness::fork_context::current_parent;
use crate::openhuman::agent_orchestration::subagent_sessions::{
    self, DurableSubagentSessionSummary, SubagentSessionStore,
};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

pub struct ListSubagentsTool;

impl ListSubagentsTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ListSubagentsTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ListSubagentsTool {
    fn name(&self) -> &str {
        "list_subagents"
    }

    fn description(&self) -> &str {
        "List active or reusable durable sub-agents owned by the current parent thread."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": [],
            "properties": {}
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let parent = match current_parent() {
            Some(parent) => parent,
            None => {
                return Ok(ToolResult::error(
                    "list_subagents called outside of an agent turn",
                ));
            }
        };
        let parent_thread_id =
            crate::openhuman::inference::provider::thread_context::current_thread_id();
        let store = SubagentSessionStore::new(parent.workspace_dir.clone());
        match subagent_sessions::list_for_parent(
            &store,
            &parent.session_id,
            parent_thread_id.as_deref(),
        ) {
            Ok(sessions) => {
                let summaries: Vec<DurableSubagentSessionSummary> = sessions
                    .iter()
                    .map(DurableSubagentSessionSummary::from)
                    .collect();
                log::debug!(
                    "[subagent_reuse] list parent_thread_id={} parent_session={} count={}",
                    parent_thread_id.as_deref().unwrap_or("none"),
                    parent.session_id,
                    summaries.len()
                );
                Ok(ToolResult::success(format!(
                    "[subagent_sessions]\n{}\n[/subagent_sessions]",
                    serde_json::to_string_pretty(&summaries).unwrap_or_else(|_| "[]".to_string())
                )))
            }
            Err(err) => Ok(ToolResult::error(format!(
                "list_subagents: failed to read sub-agent sessions: {err}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_has_no_required_fields() {
        let schema = ListSubagentsTool::new().parameters_schema();
        assert!(schema
            .get("required")
            .and_then(|v| v.as_array())
            .expect("required")
            .is_empty());
    }

    #[test]
    fn summary_projection_does_not_include_history() {
        let raw = serde_json::to_string(&DurableSubagentSessionSummary {
            subagent_session_id: "subsess-1".into(),
            parent_thread_id: Some("thread-1".into()),
            worker_thread_id: Some("worker-1".into()),
            agent_id: "researcher".into(),
            display_name: Some("Researcher".into()),
            toolkit: None,
            model: Some("agentic-v1".into()),
            sandbox_mode: "workspace".into(),
            action_root: None,
            task_key: "task".into(),
            task_title: "Task".into(),
            current_task_id: Some("sub-1".into()),
            status: subagent_sessions::DurableSubagentStatus::Idle,
            reusable: true,
            latest_error: None,
            created_at: "now".into(),
            updated_at: "now".into(),
            last_used_at: "now".into(),
        })
        .unwrap();
        assert!(!raw.contains("latestHistory"));
    }
}
