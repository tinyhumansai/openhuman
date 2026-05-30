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
                        worker_thread_id: None,
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
        model_override: None,
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
#[path = "spawn_parallel_agents_tests.rs"]
mod tests;
