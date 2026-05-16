//! Tool: `spawn_parallel_agents` — fan out independent sub-agent tasks.

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::agent::harness::definition::{AgentDefinition, AgentDefinitionRegistry};
use crate::openhuman::agent::harness::fork_context::current_parent;
use crate::openhuman::agent::harness::subagent_runner::{run_subagent, SubagentRunOptions};
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::json;

pub struct SpawnParallelAgentsTool;

impl SpawnParallelAgentsTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SpawnParallelAgentsTool {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ParallelAgentTask {
    agent_id: String,
    prompt: String,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    toolkit: Option<String>,
    #[serde(default)]
    ownership: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ParallelAgentResult {
    task_id: String,
    agent_id: String,
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ownership: Option<String>,
    elapsed_ms: u64,
    iterations: u32,
}

#[async_trait]
impl Tool for SpawnParallelAgentsTool {
    fn name(&self) -> &str {
        "spawn_parallel_agents"
    }

    fn description(&self) -> &str {
        "Run two or more independent sub-agent tasks concurrently and collect their results. \
         Use only when tasks have clear non-overlapping ownership or read-only scopes. Each task \
         has `{agent_id, prompt, context?, toolkit?, ownership?}`; include `ownership` for file, \
         module, or responsibility boundaries so workers do not overlap."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let agent_ids: Vec<String> = AgentDefinitionRegistry::global()
            .map(|reg| reg.list().iter().map(|d| d.id.clone()).collect())
            .unwrap_or_default();
        let agent_id_schema = if agent_ids.is_empty() {
            json!({ "type": "string" })
        } else {
            json!({ "type": "string", "enum": agent_ids })
        };
        json!({
            "type": "object",
            "required": ["tasks"],
            "properties": {
                "tasks": {
                    "type": "array",
                    "minItems": 2,
                    "items": {
                        "type": "object",
                        "required": ["agent_id", "prompt"],
                        "properties": {
                            "agent_id": agent_id_schema,
                            "prompt": { "type": "string" },
                            "context": { "type": "string" },
                            "toolkit": { "type": "string" },
                            "ownership": {
                                "type": "string",
                                "description": "Disjoint file/module/responsibility boundary for this worker."
                            }
                        }
                    }
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        tracing::debug!("[spawn_parallel_agents] execute entry");
        let tasks_value = args.get("tasks").cloned().ok_or_else(|| {
            tracing::debug!("[spawn_parallel_agents] missing_tasks_parameter");
            anyhow::anyhow!("Missing 'tasks' parameter")
        })?;
        let tasks: Vec<ParallelAgentTask> = serde_json::from_value(tasks_value).map_err(|e| {
            tracing::debug!(error = %e, "[spawn_parallel_agents] invalid_tasks_array");
            anyhow::anyhow!("Invalid tasks array: {e}")
        })?;

        if tasks.len() < 2 {
            tracing::debug!(
                task_count = tasks.len(),
                "[spawn_parallel_agents] rejected_too_few_tasks"
            );
            return Ok(ToolResult::error(
                "spawn_parallel_agents requires at least two tasks",
            ));
        }

        let parent = match current_parent() {
            Some(parent) => parent,
            None => {
                tracing::debug!("[spawn_parallel_agents] rejected_outside_agent_turn");
                return Ok(ToolResult::error(
                    "spawn_parallel_agents called outside of an agent turn",
                ));
            }
        };
        let max_parallel = parent.agent_config.max_parallel_tools.max(2);
        tracing::debug!(
            parent_session = %parent.session_id,
            task_count = tasks.len(),
            max_parallel,
            "[spawn_parallel_agents] validated_parent_context"
        );
        if tasks.len() > max_parallel {
            tracing::debug!(
                parent_session = %parent.session_id,
                task_count = tasks.len(),
                max_parallel,
                "[spawn_parallel_agents] rejected_too_many_tasks"
            );
            return Ok(ToolResult::error(format!(
                "spawn_parallel_agents received {} tasks but max_parallel_tools is {}",
                tasks.len(),
                max_parallel
            )));
        }

        let registry = match AgentDefinitionRegistry::global() {
            Some(registry) => registry,
            None => {
                tracing::debug!("[spawn_parallel_agents] registry_unavailable");
                return Ok(ToolResult::error(
                    "spawn_parallel_agents: AgentDefinitionRegistry has not been initialised",
                ));
            }
        };

        let parent_session = parent.session_id.clone();
        let progress_sink = parent.on_progress.clone();
        let mut immediate_results = Vec::new();
        let mut prepared = Vec::new();

        for task in tasks {
            let agent_id = task.agent_id.trim().to_string();
            let prompt = task.prompt.trim().to_string();
            let task_id = format!("sub-{}", uuid::Uuid::new_v4());
            if agent_id.is_empty() || prompt.is_empty() {
                tracing::debug!(
                    parent_session = %parent_session,
                    task_id = %task_id,
                    agent_id = %agent_id,
                    "[spawn_parallel_agents] invalid_task_missing_agent_or_prompt"
                );
                immediate_results.push(ParallelAgentResult {
                    task_id,
                    agent_id,
                    success: false,
                    output: None,
                    error: Some("agent_id and prompt are required".to_string()),
                    ownership: task.ownership,
                    elapsed_ms: 0,
                    iterations: 0,
                });
                continue;
            }

            let Some(definition) = registry.get(&agent_id).cloned() else {
                tracing::debug!(
                    parent_session = %parent_session,
                    task_id = %task_id,
                    agent_id = %agent_id,
                    "[spawn_parallel_agents] invalid_task_unknown_agent"
                );
                immediate_results.push(ParallelAgentResult {
                    task_id,
                    agent_id: agent_id.clone(),
                    success: false,
                    output: None,
                    error: Some(format!("unknown agent_id '{agent_id}'")),
                    ownership: task.ownership,
                    elapsed_ms: 0,
                    iterations: 0,
                });
                continue;
            };

            if definition.id == "integrations_agent"
                && task
                    .toolkit
                    .as_ref()
                    .map(|s| s.trim().is_empty())
                    .unwrap_or(true)
            {
                tracing::debug!(
                    parent_session = %parent_session,
                    task_id = %task_id,
                    agent_id = %agent_id,
                    "[spawn_parallel_agents] invalid_task_missing_toolkit"
                );
                immediate_results.push(ParallelAgentResult {
                    task_id,
                    agent_id,
                    success: false,
                    output: None,
                    error: Some("integrations_agent requires toolkit".to_string()),
                    ownership: task.ownership,
                    elapsed_ms: 0,
                    iterations: 0,
                });
                continue;
            }

            let prompt = with_ownership_boundary(&prompt, task.ownership.as_deref());
            tracing::debug!(
                parent_session = %parent_session,
                task_id = %task_id,
                agent_id = %definition.id,
                prompt_chars = prompt.chars().count(),
                has_ownership = task.ownership.as_deref().map(str::trim).filter(|s| !s.is_empty()).is_some(),
                "[spawn_parallel_agents] publishing_subagent_spawned"
            );
            publish_global(DomainEvent::SubagentSpawned {
                parent_session: parent_session.clone(),
                agent_id: definition.id.clone(),
                mode: "typed".to_string(),
                task_id: task_id.clone(),
                prompt_chars: prompt.chars().count(),
            });
            if let Some(ref tx) = progress_sink {
                if let Err(err) = tx
                    .send(AgentProgress::SubagentSpawned {
                        agent_id: definition.id.clone(),
                        task_id: task_id.clone(),
                        mode: "typed".to_string(),
                        dedicated_thread: false,
                        prompt_chars: prompt.chars().count(),
                    })
                    .await
                {
                    tracing::debug!(
                        parent_session = %parent_session,
                        task_id = %task_id,
                        agent_id = %definition.id,
                        error = %err,
                        "[spawn_parallel_agents] progress_send_failed spawned"
                    );
                }
            }
            prepared.push((definition, prompt, task, task_id));
        }
        tracing::debug!(
            parent_session = %parent_session,
            prepared_count = prepared.len(),
            immediate_count = immediate_results.len(),
            "[spawn_parallel_agents] prepared_tasks"
        );

        let futures = prepared
            .into_iter()
            .map(|(definition, prompt, task, task_id)| async move {
                run_one_parallel_task(definition, prompt, task, task_id).await
            });
        let mut results = immediate_results;
        for result in join_all(futures).await {
            match &result {
                ParallelAgentResult {
                    success: true,
                    agent_id,
                    task_id,
                    elapsed_ms,
                    iterations,
                    output,
                    ..
                } => {
                    tracing::debug!(
                        parent_session = %parent_session,
                        task_id = %task_id,
                        agent_id = %agent_id,
                        elapsed_ms = *elapsed_ms,
                        iterations = *iterations,
                        "[spawn_parallel_agents] publishing_subagent_completed"
                    );
                    publish_global(DomainEvent::SubagentCompleted {
                        parent_session: parent_session.clone(),
                        task_id: task_id.clone(),
                        agent_id: agent_id.clone(),
                        elapsed_ms: *elapsed_ms,
                        output_chars: output.as_ref().map(|s| s.chars().count()).unwrap_or(0),
                        iterations: *iterations as usize,
                    });
                    if let Some(ref tx) = progress_sink {
                        if let Err(err) = tx
                            .send(AgentProgress::SubagentCompleted {
                                agent_id: agent_id.clone(),
                                task_id: task_id.clone(),
                                elapsed_ms: *elapsed_ms,
                                iterations: *iterations,
                                output_chars: output
                                    .as_ref()
                                    .map(|s| s.chars().count())
                                    .unwrap_or(0),
                            })
                            .await
                        {
                            tracing::debug!(
                                parent_session = %parent_session,
                                task_id = %task_id,
                                agent_id = %agent_id,
                                error = %err,
                                "[spawn_parallel_agents] progress_send_failed completed"
                            );
                        }
                    }
                }
                ParallelAgentResult {
                    success: false,
                    agent_id,
                    task_id,
                    error,
                    ..
                } => {
                    let message = error
                        .clone()
                        .unwrap_or_else(|| "unknown failure".to_string());
                    tracing::debug!(
                        parent_session = %parent_session,
                        task_id = %task_id,
                        agent_id = %agent_id,
                        error = %message,
                        "[spawn_parallel_agents] publishing_subagent_failed"
                    );
                    publish_global(DomainEvent::SubagentFailed {
                        parent_session: parent_session.clone(),
                        task_id: task_id.clone(),
                        agent_id: agent_id.clone(),
                        error: message.clone(),
                    });
                    if let Some(ref tx) = progress_sink {
                        if let Err(err) = tx
                            .send(AgentProgress::SubagentFailed {
                                agent_id: agent_id.clone(),
                                task_id: task_id.clone(),
                                error: message,
                            })
                            .await
                        {
                            tracing::debug!(
                                parent_session = %parent_session,
                                task_id = %task_id,
                                agent_id = %agent_id,
                                error = %err,
                                "[spawn_parallel_agents] progress_send_failed failed"
                            );
                        }
                    }
                }
            }
            results.push(result);
        }

        let failures = results.iter().filter(|r| !r.success).count();
        tracing::debug!(
            parent_session = %parent_session,
            total = results.len(),
            succeeded = results.len().saturating_sub(failures),
            failed = failures,
            "[spawn_parallel_agents] execute exit"
        );
        Ok(ToolResult::success(
            serde_json::to_string_pretty(&json!({
                "parallel_agents": {
                    "total": results.len(),
                    "succeeded": results.len() - failures,
                    "failed": failures,
                    "results": results,
                }
            }))
            .unwrap_or_else(|_| "{}".to_string()),
        ))
    }
}

async fn run_one_parallel_task(
    definition: AgentDefinition,
    prompt: String,
    task: ParallelAgentTask,
    task_id: String,
) -> ParallelAgentResult {
    let started = std::time::Instant::now();
    tracing::debug!(
        task_id = %task_id,
        agent_id = %definition.id,
        toolkit = task.toolkit.as_deref().unwrap_or(""),
        context_chars = task.context.as_ref().map(|s| s.chars().count()).unwrap_or(0),
        prompt_chars = prompt.chars().count(),
        "[spawn_parallel_agents] task_start"
    );
    let options = SubagentRunOptions {
        skill_filter_override: None,
        toolkit_override: task.toolkit.clone(),
        context: task.context.clone(),
        task_id: Some(task_id.clone()),
        worker_thread_id: None,
    };
    match run_subagent(&definition, &prompt, options).await {
        Ok(outcome) => {
            tracing::debug!(
                task_id = %outcome.task_id,
                agent_id = %outcome.agent_id,
                elapsed_ms = outcome.elapsed.as_millis() as u64,
                iterations = outcome.iterations,
                output_chars = outcome.output.chars().count(),
                "[spawn_parallel_agents] task_success"
            );
            ParallelAgentResult {
                task_id: outcome.task_id,
                agent_id: outcome.agent_id,
                success: true,
                output: Some(outcome.output),
                error: None,
                ownership: task.ownership,
                elapsed_ms: outcome.elapsed.as_millis() as u64,
                iterations: outcome.iterations as u32,
            }
        }
        Err(err) => {
            tracing::debug!(
                task_id = %task_id,
                agent_id = %definition.id,
                elapsed_ms = started.elapsed().as_millis() as u64,
                error = %err,
                "[spawn_parallel_agents] task_error"
            );
            ParallelAgentResult {
                task_id,
                agent_id: definition.id,
                success: false,
                output: None,
                error: Some(err.to_string()),
                ownership: task.ownership,
                elapsed_ms: started.elapsed().as_millis() as u64,
                iterations: 0,
            }
        }
    }
}

fn with_ownership_boundary(prompt: &str, ownership: Option<&str>) -> String {
    match ownership.map(str::trim).filter(|s| !s.is_empty()) {
        Some(boundary) => format!(
            "[Ownership Boundary]\n{boundary}\n\n[Task]\n{prompt}\n\nDo not work outside the ownership boundary unless the parent explicitly asks you to."
        ),
        None => prompt.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::harness::fork_context::{
        with_parent_context, ParentExecutionContext,
    };
    use crate::openhuman::context::prompt::ToolCallFormat;
    use crate::openhuman::memory::{
        Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts,
    };
    use crate::openhuman::providers::{ChatRequest, ChatResponse, Provider};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use tokio::time::{sleep, Duration};

    #[test]
    fn metadata_methods_expose_execute_permission_and_schema() {
        let tool = SpawnParallelAgentsTool::default();
        assert_eq!(tool.name(), "spawn_parallel_agents");
        assert!(tool.description().contains("independent sub-agent tasks"));
        assert_eq!(tool.permission_level(), PermissionLevel::Execute);
        let schema = tool.parameters_schema();
        assert_eq!(schema["required"][0], "tasks");
        assert_eq!(schema["properties"]["tasks"]["minItems"], 2);
    }

    #[test]
    fn ownership_boundary_is_prepended_when_present() {
        let prompt = with_ownership_boundary("implement tests", Some("files: src/foo.rs"));
        assert!(prompt.starts_with("[Ownership Boundary]"));
        assert!(prompt.contains("files: src/foo.rs"));
        assert!(prompt.contains("[Task]\nimplement tests"));
    }

    #[tokio::test]
    async fn rejects_single_task() {
        let tool = SpawnParallelAgentsTool::new();
        let result = tool
            .execute(json!({
                "tasks": [{ "agent_id": "researcher", "prompt": "only one" }]
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("at least two"));
    }

    #[tokio::test]
    async fn rejects_missing_or_invalid_tasks_before_parent_lookup() {
        let tool = SpawnParallelAgentsTool::new();

        let missing = tool.execute(json!({})).await.expect_err("missing tasks");
        assert!(missing.to_string().contains("Missing 'tasks'"));

        let invalid = tool
            .execute(json!({ "tasks": "not an array" }))
            .await
            .expect_err("invalid tasks");
        assert!(invalid.to_string().contains("Invalid tasks array"));
    }

    #[tokio::test]
    async fn rejects_two_tasks_outside_agent_turn() {
        let tool = SpawnParallelAgentsTool::new();
        let result = tool
            .execute(json!({
                "tasks": [
                    { "agent_id": "researcher", "prompt": "one" },
                    { "agent_id": "planner", "prompt": "two" }
                ]
            }))
            .await
            .expect("tool result");
        assert!(result.is_error);
        assert!(result.output().contains("outside of an agent turn"));
    }

    struct NoopProvider;

    #[async_trait]
    impl Provider for NoopProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            Ok("ok".into())
        }

        async fn chat(
            &self,
            _request: ChatRequest<'_>,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<ChatResponse> {
            Ok(ChatResponse {
                text: Some("ok".into()),
                tool_calls: Vec::new(),
                usage: None,
            })
        }
    }

    struct ConcurrentProvider {
        active: AtomicUsize,
        max_active: AtomicUsize,
    }

    impl ConcurrentProvider {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                active: AtomicUsize::new(0),
                max_active: AtomicUsize::new(0),
            })
        }

        fn max_active(&self) -> usize {
            self.max_active.load(Ordering::SeqCst)
        }

        fn observe_active(&self, current: usize) {
            let mut observed = self.max_active.load(Ordering::SeqCst);
            while current > observed {
                match self.max_active.compare_exchange(
                    observed,
                    current,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break,
                    Err(next) => observed = next,
                }
            }
        }
    }

    #[async_trait]
    impl Provider for ConcurrentProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            Ok("ok".into())
        }

        async fn chat(
            &self,
            _request: ChatRequest<'_>,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<ChatResponse> {
            let current = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.observe_active(current);
            sleep(Duration::from_millis(50)).await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok(ChatResponse {
                text: Some("parallel ok".into()),
                tool_calls: Vec::new(),
                usage: None,
            })
        }
    }

    struct NoopMemory;

    #[async_trait]
    impl Memory for NoopMemory {
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

        fn name(&self) -> &str {
            "noop"
        }
    }

    fn parent_context_with_provider(
        max_parallel_tools: usize,
        provider: Arc<dyn Provider>,
    ) -> ParentExecutionContext {
        let agent_config = crate::openhuman::config::AgentConfig {
            max_parallel_tools,
            ..Default::default()
        };
        ParentExecutionContext {
            provider,
            all_tools: Arc::new(Vec::new()),
            all_tool_specs: Arc::new(Vec::new()),
            model_name: "test-model".into(),
            temperature: 0.2,
            workspace_dir: std::env::temp_dir(),
            memory: Arc::new(NoopMemory),
            agent_config,
            skills: Arc::new(Vec::new()),
            memory_context: Arc::new(None),
            session_id: "session-test".into(),
            channel: "test".into(),
            connected_integrations: Vec::new(),
            tool_call_format: ToolCallFormat::PFormat,
            session_key: "0_test".into(),
            session_parent_prefix: None,
            on_progress: None,
        }
    }

    fn parent_context(max_parallel_tools: usize) -> ParentExecutionContext {
        parent_context_with_provider(max_parallel_tools, Arc::new(NoopProvider))
    }

    #[tokio::test]
    async fn rejects_more_tasks_than_parent_parallel_limit() {
        let tool = SpawnParallelAgentsTool::new();
        let parent = parent_context(2);
        let result = with_parent_context(parent, async {
            tool.execute(json!({
                "tasks": [
                    { "agent_id": "researcher", "prompt": "one" },
                    { "agent_id": "planner", "prompt": "two" },
                    { "agent_id": "critic", "prompt": "three" }
                ]
            }))
            .await
        })
        .await
        .expect("tool result");
        assert!(result.is_error);
        assert!(result.output().contains("max_parallel_tools"));
    }

    #[tokio::test]
    async fn collects_immediate_task_validation_failures() {
        let _ = AgentDefinitionRegistry::init_global_builtins();
        let tool = SpawnParallelAgentsTool::new();
        let parent = parent_context(4);

        let result = with_parent_context(parent, async {
            tool.execute(json!({
                "tasks": [
                    { "agent_id": " ", "prompt": "missing agent", "ownership": "files: none" },
                    { "agent_id": "__missing_agent__", "prompt": "unknown agent" },
                    { "agent_id": "integrations_agent", "prompt": "needs toolkit" }
                ]
            }))
            .await
        })
        .await
        .expect("tool result");

        assert!(!result.is_error, "{}", result.output());
        let body: serde_json::Value = serde_json::from_str(&result.output()).expect("json output");
        assert_eq!(body["parallel_agents"]["total"], 3);
        assert_eq!(body["parallel_agents"]["failed"], 3);
        let errors = body["parallel_agents"]["results"]
            .as_array()
            .expect("results")
            .iter()
            .map(|result| result["error"].as_str().unwrap_or_default())
            .collect::<Vec<_>>();
        assert!(errors
            .iter()
            .any(|error| error.contains("agent_id and prompt")));
        assert!(errors
            .iter()
            .any(|error| error.contains("unknown agent_id")));
        assert!(errors
            .iter()
            .any(|error| error.contains("requires toolkit")));
    }

    // After upstream PR #1858 (`feat(ai): unified per-workload provider
    // routing + chat-provider factory`), the subagent runner resolves a
    // real provider via `resolve_subagent_provider` using the loaded
    // `Config`'s workload routing instead of inheriting `parent.provider`
    // for `ModelSpec::Hint` agents like `researcher` / `planner`. That
    // bypasses this test's mock `ConcurrentProvider`: on CI the resolved
    // OpenHuman backend is unreachable so subagents fail with
    // `succeeded == 0`; locally a configured cloud provider may answer
    // with text that isn't `"parallel ok"`. The right fix is a
    // workspace-isolated test fixture that forces the workload factory
    // to fall back to the parent provider — tracked as a follow-up.
    #[ignore = "subagent_runner provider routing bypasses mock provider after upstream PR #1858"]
    #[tokio::test]
    async fn runs_valid_tasks_concurrently_and_collects_successes() {
        let _ = AgentDefinitionRegistry::init_global_builtins();
        let tool = SpawnParallelAgentsTool::new();
        let provider_impl = ConcurrentProvider::new();
        let provider: Arc<dyn Provider> = provider_impl.clone();
        let parent = parent_context_with_provider(4, provider);

        let result = with_parent_context(parent, async {
            tool.execute(json!({
                "tasks": [
                    {
                        "agent_id": "researcher",
                        "prompt": "summarize alpha",
                        "ownership": "files: alpha.rs"
                    },
                    {
                        "agent_id": "planner",
                        "prompt": "plan beta",
                        "ownership": "files: beta.rs"
                    }
                ]
            }))
            .await
        })
        .await
        .expect("tool result");

        assert!(!result.is_error, "{}", result.output());
        let body: serde_json::Value = serde_json::from_str(&result.output()).expect("json output");
        assert_eq!(body["parallel_agents"]["total"], 2);
        assert_eq!(body["parallel_agents"]["succeeded"], 2);
        assert_eq!(body["parallel_agents"]["failed"], 0);
        let results = body["parallel_agents"]["results"]
            .as_array()
            .expect("results");
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|result| result["success"] == true));
        assert!(results
            .iter()
            .all(|result| result["output"].as_str() == Some("parallel ok")));
        assert!(
            provider_impl.max_active() >= 2,
            "expected overlapping provider calls, max_active={}",
            provider_impl.max_active()
        );
    }
}
