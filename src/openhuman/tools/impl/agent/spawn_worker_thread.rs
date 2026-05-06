//! Tool: `spawn_worker_thread` — spawn a dedicated worker thread for a complex delegated task.
//!
//! Unlike `spawn_subagent`, which collapses sub-agent work into a single
//! tool result in the current thread, `spawn_worker_thread` creates a new
//! persisted thread with label `worker`. The sub-agent's full transcript
//! is recorded into that thread, and the parent receives a compact
//! reference (worker thread id) instead of the full output.
//!
//! Worker threads carry a hard cap on depth: a worker thread cannot spawn
//! another worker thread.

use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::current_parent;
use crate::openhuman::agent::harness::subagent_runner::{run_subagent, SubagentRunOptions};
use crate::openhuman::memory::conversations::{
    self, ConversationMessage, CreateConversationThread,
};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

/// Spawns a sub-agent in a dedicated worker thread.
pub struct SpawnWorkerThreadTool;

impl Default for SpawnWorkerThreadTool {
    fn default() -> Self {
        Self::new()
    }
}

impl SpawnWorkerThreadTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for SpawnWorkerThreadTool {
    fn name(&self) -> &str {
        "spawn_worker_thread"
    }

    fn description(&self) -> &str {
        "Spawn a dedicated worker thread for a complex delegated task. \
         Use this when the task is long or involves many steps that would \
         clutter the current conversation. The sub-agent runs in a fresh \
         thread labeled 'worker', and you receive the thread ID and a \
         summary. Worker threads cannot spawn other worker threads."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let agent_ids: Vec<String> = AgentDefinitionRegistry::global()
            .map(|reg| reg.list().iter().map(|d| d.id.clone()).collect())
            .unwrap_or_default();

        let agent_id_schema = if agent_ids.is_empty() {
            json!({
                "type": "string",
                "description": "Sub-agent id (e.g. code_executor, researcher, planner)."
            })
        } else {
            json!({
                "type": "string",
                "enum": agent_ids,
                "description": "Sub-agent id from the registry."
            })
        };

        json!({
            "type": "object",
            "required": ["agent_id", "prompt", "task_title"],
            "properties": {
                "agent_id": agent_id_schema,
                "prompt": {
                    "type": "string",
                    "description": "Clear, specific instruction for the sub-agent. The sub-agent has no memory of the parent's conversation, so include all context the sub-agent needs to act."
                },
                "task_title": {
                    "type": "string",
                    "description": "A short, descriptive title for the worker thread (e.g. 'Researching Rust async patterns')."
                },
                "context": {
                    "type": "string",
                    "description": "Optional context blob from prior task results. Rendered as a `[Context]` block before the prompt."
                },
                "toolkit": {
                    "type": "string",
                    "description": "Composio toolkit slug to scope this spawn to (e.g. `gmail`, `notion`)."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let started = std::time::Instant::now();

        let agent_id = args
            .get("agent_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let task_title = args
            .get("task_title")
            .and_then(|v| v.as_str())
            .unwrap_or("Worker Task")
            .to_string();
        let context = args
            .get("context")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let toolkit_override = args
            .get("toolkit")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        if agent_id.is_empty() || prompt.is_empty() {
            tracing::warn!(
                agent_id = %agent_id,
                prompt_empty = prompt.is_empty(),
                "[spawn_worker_thread] rejected: agent_id and prompt are required"
            );
            return Ok(ToolResult::error("agent_id and prompt are required"));
        }

        let parent = current_parent().ok_or_else(|| anyhow::anyhow!("no parent context"))?;

        // ── Depth Guard ────────────────────────────────────────────────
        // Check if the current thread is already a worker thread.
        let current_thread_id = crate::openhuman::providers::thread_context::current_thread_id()
            .unwrap_or_else(|| "unknown".to_string());

        tracing::info!(
            agent_id = %agent_id,
            task_title = %task_title,
            current_thread_id = %current_thread_id,
            toolkit_override = ?toolkit_override,
            has_context = context.is_some(),
            "[spawn_worker_thread] invoked"
        );

        let threads = conversations::list_threads(parent.workspace_dir.clone())
            .map_err(|e| anyhow::anyhow!(e))?;
        if let Some(current_thread) = threads.iter().find(|t| t.id == current_thread_id) {
            if current_thread.labels.contains(&"worker".to_string())
                || current_thread.parent_thread_id.is_some()
            {
                tracing::warn!(
                    agent_id = %agent_id,
                    current_thread_id = %current_thread_id,
                    is_worker_label = current_thread.labels.contains(&"worker".to_string()),
                    has_parent_thread_id = current_thread.parent_thread_id.is_some(),
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "[spawn_worker_thread] depth guard blocked spawn from worker thread"
                );
                return Ok(ToolResult::error("Worker threads cannot spawn other worker threads. Depth is capped at 1. Use spawn_subagent for inline delegation instead."));
            }
        }

        let registry = AgentDefinitionRegistry::global()
            .ok_or_else(|| anyhow::anyhow!("AgentDefinitionRegistry not initialised"))?;

        let definition = registry
            .get(&agent_id)
            .ok_or_else(|| anyhow::anyhow!("agent_id '{}' not found", agent_id))?;

        // ── Create Worker Thread ───────────────────────────────────────
        let worker_thread_id = format!("worker-{}", uuid::Uuid::new_v4());
        let now = chrono::Utc::now().to_rfc3339();

        conversations::ensure_thread(
            parent.workspace_dir.clone(),
            CreateConversationThread {
                id: worker_thread_id.clone(),
                title: task_title.clone(),
                created_at: now.clone(),
                parent_thread_id: Some(current_thread_id.clone()),
                labels: Some(vec!["worker".to_string()]),
            },
        )
        .map_err(|e| anyhow::anyhow!(e))?;

        tracing::info!(
            agent_id = %agent_id,
            worker_thread_id = %worker_thread_id,
            parent_thread_id = %current_thread_id,
            task_title = %task_title,
            created_at = %now,
            "[spawn_worker_thread] created worker thread"
        );

        // Append initial user message to the worker thread
        conversations::append_message(
            parent.workspace_dir.clone(),
            &worker_thread_id,
            ConversationMessage {
                id: format!("user:{}", uuid::Uuid::new_v4()),
                content: prompt.clone(),
                message_type: "text".to_string(),
                extra_metadata: json!({
                    "scope": "worker_thread",
                    "agent_id": agent_id,
                }),
                sender: "user".to_string(),
                created_at: now,
            },
        )
        .map_err(|e| anyhow::anyhow!(e))?;

        // We don't have an easy way to append a system message to the parent
        // thread here without triggering a re-render of the history the model
        // sees. Instead, we return the info in the tool result.

        // ── Run Subagent ──────────────────────────────────────────────
        let options = SubagentRunOptions {
            skill_filter_override: None,
            toolkit_override,
            context,
            task_id: None,
            worker_thread_id: Some(worker_thread_id.clone()),
        };

        tracing::debug!(
            agent_id = %agent_id,
            worker_thread_id = %worker_thread_id,
            "[spawn_worker_thread] dispatching run_subagent"
        );

        match run_subagent(definition, &prompt, options).await {
            Ok(outcome) => {
                tracing::info!(
                    agent_id = %agent_id,
                    worker_thread_id = %worker_thread_id,
                    task_id = %outcome.task_id,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "[spawn_worker_thread] completed successfully"
                );
                let parent_visible = format!(
                    "Spawned worker thread `{worker_thread_id}` for the task: {task_title}. \
                     The sub-agent has completed its work. You can find the full transcript \
                     in the worker thread.\n\n\
                     [worker_thread_ref]\n{}\n[/worker_thread_ref]",
                    json!({
                        "thread_id": worker_thread_id,
                        "label": "worker",
                        "agent_id": agent_id,
                        "task_id": outcome.task_id,
                        "status": "completed"
                    })
                );
                Ok(ToolResult::success(parent_visible))
            }
            Err(err) => {
                tracing::error!(
                    agent_id = %agent_id,
                    worker_thread_id = %worker_thread_id,
                    error = %err,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "[spawn_worker_thread] execution failed"
                );
                Ok(ToolResult::error(format!(
                    "Worker thread execution failed: {err}"
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::harness::definition::{
        AgentDefinition, DefinitionSource, ModelSpec, PromptSource, SandboxMode, ToolScope,
    };
    use crate::openhuman::agent::harness::fork_context::with_parent_context;
    use crate::openhuman::agent::harness::ParentExecutionContext;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::TempDir;

    struct MockProvider;
    #[async_trait]
    impl crate::openhuman::providers::Provider for MockProvider {
        async fn chat_with_system(
            &self,
            _: Option<&str>,
            _: &str,
            _: &str,
            _: f64,
        ) -> anyhow::Result<String> {
            Ok("".into())
        }
        async fn chat(
            &self,
            _: crate::openhuman::providers::ChatRequest<'_>,
            _: &str,
            _: f64,
        ) -> anyhow::Result<crate::openhuman::providers::ChatResponse> {
            Ok(crate::openhuman::providers::ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
            })
        }
        fn supports_native_tools(&self) -> bool {
            true
        }
    }

    struct MockMemory;
    #[async_trait]
    impl crate::openhuman::memory::Memory for MockMemory {
        async fn store(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: crate::openhuman::memory::MemoryCategory,
            _: Option<&str>,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn recall(
            &self,
            _: &str,
            _: usize,
            _: crate::openhuman::memory::RecallOpts<'_>,
        ) -> anyhow::Result<Vec<crate::openhuman::memory::MemoryEntry>> {
            Ok(vec![])
        }
        async fn get(
            &self,
            _: &str,
            _: &str,
        ) -> anyhow::Result<Option<crate::openhuman::memory::MemoryEntry>> {
            Ok(None)
        }
        async fn list(
            &self,
            _: Option<&str>,
            _: Option<&crate::openhuman::memory::MemoryCategory>,
            _: Option<&str>,
        ) -> anyhow::Result<Vec<crate::openhuman::memory::MemoryEntry>> {
            Ok(vec![])
        }
        async fn forget(&self, _: &str, _: &str) -> anyhow::Result<bool> {
            Ok(true)
        }
        async fn namespace_summaries(
            &self,
        ) -> anyhow::Result<Vec<crate::openhuman::memory::NamespaceSummary>> {
            Ok(vec![])
        }
        async fn count(&self) -> anyhow::Result<usize> {
            Ok(0)
        }
        async fn health_check(&self) -> bool {
            true
        }
        fn name(&self) -> &str {
            "mock"
        }
    }

    fn test_parent_ctx(workspace_dir: PathBuf) -> ParentExecutionContext {
        ParentExecutionContext {
            session_id: "test".into(),
            session_key: "test".into(),
            session_parent_prefix: None,
            model_name: "test".into(),
            temperature: 0.4,
            workspace_dir,
            provider: Arc::new(MockProvider),
            memory: Arc::new(MockMemory),
            channel: "test".into(),
            all_tools: Arc::new(vec![]),
            all_tool_specs: Arc::new(vec![]),
            skills: Arc::new(vec![]),
            memory_context: None,
            connected_integrations: vec![],
            composio_client: None,
            on_progress: None,
            agent_config: crate::openhuman::config::AgentConfig::default(),
            tool_call_format: crate::openhuman::context::prompt::ToolCallFormat::Native,
        }
    }

    #[tokio::test]
    async fn rejects_if_already_worker_thread() {
        let temp = TempDir::new().unwrap();
        let thread_id = "worker-123";
        conversations::ensure_thread(
            temp.path().to_path_buf(),
            CreateConversationThread {
                id: thread_id.to_string(),
                title: "Worker".into(),
                created_at: "now".into(),
                parent_thread_id: None,
                labels: Some(vec!["worker".to_string()]),
            },
        )
        .unwrap();

        crate::openhuman::providers::thread_context::with_thread_id(thread_id.to_string(), async {
            let parent = test_parent_ctx(temp.path().to_path_buf());
            with_parent_context(parent, async {
                let tool = SpawnWorkerThreadTool::new();
                let result = tool
                    .execute(json!({
                        "agent_id": "researcher",
                        "prompt": "do it",
                        "task_title": "Task"
                    }))
                    .await
                    .unwrap();

                assert!(result.is_error);
                assert!(result
                    .output()
                    .contains("cannot spawn other worker threads"));
            })
            .await;
        })
        .await;
    }

    #[tokio::test]
    async fn rejects_if_has_parent_thread_id() {
        let temp = TempDir::new().unwrap();
        let thread_id = "sub-123";
        conversations::ensure_thread(
            temp.path().to_path_buf(),
            CreateConversationThread {
                id: thread_id.to_string(),
                title: "Sub".into(),
                created_at: "now".into(),
                parent_thread_id: Some("parent".into()),
                labels: None,
            },
        )
        .unwrap();

        crate::openhuman::providers::thread_context::with_thread_id(thread_id.to_string(), async {
            let parent = test_parent_ctx(temp.path().to_path_buf());
            with_parent_context(parent, async {
                let tool = SpawnWorkerThreadTool::new();
                let result = tool
                    .execute(json!({
                        "agent_id": "researcher",
                        "prompt": "do it",
                        "task_title": "Task"
                    }))
                    .await
                    .unwrap();

                assert!(result.is_error);
                assert!(result
                    .output()
                    .contains("cannot spawn other worker threads"));
            })
            .await;
        })
        .await;
    }
}
