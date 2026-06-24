//! Tool: `close_subagent` - retire a reusable durable sub-agent session.

use crate::openhuman::agent::harness::fork_context::current_parent;
use crate::openhuman::agent_orchestration::subagent_sessions::SubagentSessionStore;
use crate::openhuman::agent_orchestration::{running_subagents, subagent_sessions};
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
            crate::openhuman::inference::provider::thread_context::current_thread_id();
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
        let cancelled =
            running_subagents::cancel_by_session(&subagent_session_id, &parent.session_id)
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
mod tests {
    use super::*;
    use crate::openhuman::agent::harness::fork_context::{
        with_parent_context, ParentExecutionContext,
    };
    use crate::openhuman::config::AgentConfig;
    use crate::openhuman::context::prompt::ToolCallFormat;
    use crate::openhuman::inference::provider::Provider;
    use crate::openhuman::memory::{
        Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts,
    };
    use std::collections::HashSet;
    use std::path::Path;
    use std::sync::Arc;

    #[tokio::test]
    async fn missing_session_id_is_rejected() {
        let res = CloseSubagentTool::new().execute(json!({})).await.unwrap();
        assert!(res.is_error);
        assert!(res.output().contains("subagent_session_id"));
    }

    #[tokio::test]
    async fn rejects_session_from_different_parent_thread() {
        let workspace = tempfile::TempDir::new().expect("workspace");
        let store = SubagentSessionStore::new(workspace.path().to_path_buf());
        let session = seed_session(&store, "thread-b");

        let res = with_parent_context(parent_context(workspace.path()), async {
            crate::openhuman::inference::provider::thread_context::with_thread_id(
                "thread-a",
                async {
                    CloseSubagentTool::new()
                        .execute(json!({
                            "subagent_session_id": session.subagent_session_id,
                        }))
                        .await
                },
            )
            .await
        })
        .await
        .unwrap();

        assert!(res.is_error);
        assert!(res.output().contains("not found for this parent thread"));
        assert!(
            subagent_sessions::find_reusable(&store, &selector("thread-b"))
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn closes_session_owned_by_current_parent_thread() {
        let workspace = tempfile::TempDir::new().expect("workspace");
        let store = SubagentSessionStore::new(workspace.path().to_path_buf());
        let session = seed_session(&store, "thread-a");

        let res = with_parent_context(parent_context(workspace.path()), async {
            crate::openhuman::inference::provider::thread_context::with_thread_id(
                "thread-a",
                async {
                    CloseSubagentTool::new()
                        .execute(json!({
                            "subagent_session_id": session.subagent_session_id,
                        }))
                        .await
                },
            )
            .await
        })
        .await
        .unwrap();

        assert!(!res.is_error, "{}", res.output());
        assert!(res.output().contains("closed=true"));
        assert!(
            subagent_sessions::find_reusable(&store, &selector("thread-a"))
                .unwrap()
                .is_none()
        );
    }

    fn seed_session(
        store: &SubagentSessionStore,
        parent_thread_id: &str,
    ) -> subagent_sessions::DurableSubagentSession {
        subagent_sessions::upsert_running(
            store,
            subagent_sessions::SubagentSessionUpsert {
                selector: selector(parent_thread_id),
                display_name: Some("Researcher".into()),
                task_title: "Task".into(),
                worker_thread_id: Some("worker-1".into()),
                task_id: "sub-1".into(),
            },
            None,
        )
        .unwrap()
    }

    fn selector(parent_thread_id: &str) -> subagent_sessions::SubagentSessionSelector {
        subagent_sessions::SubagentSessionSelector {
            parent_session: "parent-session".into(),
            parent_thread_id: Some(parent_thread_id.into()),
            agent_id: "researcher".into(),
            toolkit: None,
            model: None,
            sandbox_mode: "workspace".into(),
            action_root: None,
            task_key: "task".into(),
        }
    }

    fn parent_context(workspace_dir: &Path) -> ParentExecutionContext {
        ParentExecutionContext {
            agent_definition_id: "orchestrator".into(),
            allowed_subagent_ids: HashSet::new(),
            provider: Arc::new(NoopProvider),
            all_tools: Arc::new(Vec::new()),
            all_tool_specs: Arc::new(Vec::new()),
            visible_tool_names: std::collections::HashSet::new(),
            model_name: "test-model".into(),
            temperature: 0.0,
            workspace_dir: workspace_dir.to_path_buf(),
            memory: Arc::new(NoopMemory),
            agent_config: AgentConfig::default(),
            workflows: Arc::new(Vec::new()),
            memory_context: Arc::new(None),
            session_id: "parent-session".into(),
            channel: "test".into(),
            connected_integrations: Vec::new(),
            tool_call_format: ToolCallFormat::Native,
            session_key: "parent-key".into(),
            session_parent_prefix: None,
            on_progress: None,
            run_queue: None,
        }
    }

    struct NoopProvider;

    #[async_trait::async_trait]
    impl Provider for NoopProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            Ok(String::new())
        }
    }

    struct NoopMemory;

    #[async_trait::async_trait]
    impl Memory for NoopMemory {
        fn name(&self) -> &str {
            "noop"
        }

        async fn store(
            &self,
            _namespace: &str,
            _key: &str,
            _content: &str,
            _category: MemoryCategory,
            _session_id: Option<&str>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn recall(
            &self,
            _query: &str,
            _limit: usize,
            _opts: RecallOpts<'_>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            Ok(Vec::new())
        }

        async fn get(&self, _namespace: &str, _key: &str) -> anyhow::Result<Option<MemoryEntry>> {
            Ok(None)
        }

        async fn list(
            &self,
            _namespace: Option<&str>,
            _category: Option<&MemoryCategory>,
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            Ok(Vec::new())
        }

        async fn forget(&self, _namespace: &str, _key: &str) -> anyhow::Result<bool> {
            Ok(false)
        }

        async fn namespace_summaries(&self) -> anyhow::Result<Vec<NamespaceSummary>> {
            Ok(Vec::new())
        }

        async fn count(&self) -> anyhow::Result<usize> {
            Ok(0)
        }

        async fn health_check(&self) -> bool {
            true
        }
    }
}
