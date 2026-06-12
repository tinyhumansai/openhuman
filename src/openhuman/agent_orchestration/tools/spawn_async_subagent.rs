//! Tool: `spawn_async_subagent` - fire-and-forget sub-agent delegation.
//!
//! Unlike `spawn_subagent`, this tool returns as soon as the child run is
//! accepted. Completion/failure is reported through normal sub-agent lifecycle
//! events and, when possible, persisted in the child worker thread.

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::{current_parent, with_parent_context};
use crate::openhuman::agent::harness::subagent_runner::{
    run_subagent, SubagentRunOptions, SubagentRunStatus,
};
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

pub struct SpawnAsyncSubagentTool;

impl SpawnAsyncSubagentTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SpawnAsyncSubagentTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SpawnAsyncSubagentTool {
    fn name(&self) -> &str {
        "spawn_async_subagent"
    }

    fn description(&self) -> &str {
        "Fire-and-forget a specialised sub-agent for low-attention background work. \
         Use sparingly, only when the user does not need the result in the current \
         response, such as best-effort memory archiving, cleanup, or background \
         investigation. Do not use for user-visible answers, code changes, external \
         service writes, financial actions, or anything that may need clarification."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let agent_ids: Vec<String> = AgentDefinitionRegistry::global()
            .map(|reg| reg.list().iter().map(|d| d.id.clone()).collect())
            .unwrap_or_default();

        let agent_id_schema = if agent_ids.is_empty() {
            json!({
                "type": "string",
                "description": "Sub-agent id (e.g. archivist, researcher, tools_agent)."
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
            "required": ["agent_id", "prompt"],
            "properties": {
                "agent_id": agent_id_schema,
                "prompt": {
                    "type": "string",
                    "description": "Clear, self-contained background instruction. Include all context needed. The sub-agent must not ask the user for clarification."
                },
                "context": {
                    "type": "string",
                    "description": "Optional context blob from prior task results. Rendered as a `[Context]` block before the prompt."
                },
                "model": {
                    "type": "string",
                    "description": "Optional exact model id for this background spawn only."
                },
                "toolkit": {
                    "type": "string",
                    "description": "Composio toolkit slug to scope this spawn to. Required when agent_id is `integrations_agent`."
                },
                "task_title": {
                    "type": "string",
                    "description": "Optional short title for the persisted background worker thread."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let agent_id = args
            .get("agent_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let context = args
            .get("context")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let model_override = args
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let toolkit_override = args
            .get("toolkit")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let task_title = args
            .get("task_title")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Background subagent")
            .to_string();

        if agent_id.is_empty() {
            return Ok(ToolResult::error(
                "spawn_async_subagent: `agent_id` is required",
            ));
        }
        if prompt.is_empty() {
            return Ok(ToolResult::error(
                "spawn_async_subagent: `prompt` is required",
            ));
        }

        let parent = match current_parent() {
            Some(parent) => parent,
            None => {
                return Ok(ToolResult::error(
                    "spawn_async_subagent called outside of an agent turn",
                ));
            }
        };

        let registry = match AgentDefinitionRegistry::global() {
            Some(registry) => registry,
            None => {
                return Ok(ToolResult::error(
                    "spawn_async_subagent: AgentDefinitionRegistry has not been initialised",
                ));
            }
        };
        let definition = match registry.get(&agent_id).cloned() {
            Some(definition) => definition,
            None => {
                let available: Vec<&str> = registry.list().iter().map(|d| d.id.as_str()).collect();
                return Ok(ToolResult::error(format!(
                    "spawn_async_subagent: unknown agent_id '{agent_id}'. Available: {}",
                    available.join(", ")
                )));
            }
        };

        if !parent.allowed_subagent_ids.contains(&definition.id) {
            log::warn!(
                "[spawn_async_subagent] blocked subagent outside allowlist parent={} requested={} allowed={:?}",
                parent.agent_definition_id,
                definition.id,
                parent.allowed_subagent_ids
            );
            return Ok(ToolResult::error(format!(
                "spawn_async_subagent: agent '{}' is not in parent agent '{}' subagents.allowlist",
                definition.id, parent.agent_definition_id
            )));
        }

        if definition.id == "integrations_agent" && toolkit_override.is_none() {
            return Ok(ToolResult::error(
                "spawn_async_subagent(integrations_agent): the `toolkit` argument is required",
            ));
        }

        let parent_session = parent.session_id.clone();
        let progress_sink = parent.on_progress.clone();
        let task_id = format!("sub-{}", uuid::Uuid::new_v4());
        let worker_thread_id =
            crate::openhuman::inference::provider::thread_context::current_thread_id().and_then(
                |parent_thread_id| {
                    super::worker_thread::create_worker_thread(
                        parent.workspace_dir.clone(),
                        &parent_thread_id,
                        &definition.id,
                        &task_title,
                        &prompt,
                    )
                    .ok()
                },
            );

        publish_global(DomainEvent::SubagentSpawned {
            parent_session: parent_session.clone(),
            agent_id: definition.id.clone(),
            mode: "async".to_string(),
            task_id: task_id.clone(),
            prompt_chars: prompt.chars().count(),
        });
        if let Some(ref tx) = progress_sink {
            let _ = tx
                .send(AgentProgress::SubagentSpawned {
                    agent_id: definition.id.clone(),
                    task_id: task_id.clone(),
                    mode: "async".to_string(),
                    dedicated_thread: worker_thread_id.is_some(),
                    prompt_chars: prompt.chars().count(),
                    worker_thread_id: worker_thread_id.clone(),
                    display_name: Some(definition.display_name().to_string()),
                })
                .await;
        }

        let background_parent = parent.clone();
        let background_definition = definition.clone();
        let background_agent_id = definition.id.clone();
        let background_task_id = task_id.clone();
        let background_parent_session = parent_session.clone();
        let background_progress = progress_sink.clone();
        let background_worker_thread_id = worker_thread_id.clone();
        let background_prompt = add_background_contract(&prompt);

        tokio::spawn(async move {
            let options = SubagentRunOptions {
                skill_filter_override: None,
                toolkit_override,
                context,
                model_override,
                task_id: Some(background_task_id.clone()),
                worker_thread_id: background_worker_thread_id.clone(),
                initial_history: None,
                checkpoint_dir: None,
                worktree_action_dir: None,
            };

            let result = with_parent_context(background_parent, async move {
                run_subagent(&background_definition, &background_prompt, options).await
            })
            .await;

            match result {
                Ok(outcome) => match outcome.status {
                    SubagentRunStatus::Completed => {
                        publish_global(DomainEvent::SubagentCompleted {
                            parent_session: background_parent_session,
                            task_id: outcome.task_id.clone(),
                            agent_id: outcome.agent_id.clone(),
                            elapsed_ms: outcome.elapsed.as_millis() as u64,
                            output_chars: outcome.output.chars().count(),
                            iterations: outcome.iterations,
                        });
                        if let Some(ref tx) = background_progress {
                            let _ = tx
                                .send(AgentProgress::SubagentCompleted {
                                    agent_id: outcome.agent_id,
                                    task_id: outcome.task_id,
                                    elapsed_ms: outcome.elapsed.as_millis() as u64,
                                    iterations: outcome.iterations as u32,
                                    output_chars: outcome.output.chars().count(),
                                })
                                .await;
                        }
                    }
                    SubagentRunStatus::AwaitingUser { question, .. } => {
                        let error = format!(
                            "async sub-agent requested user clarification and was not continued: {question}"
                        );
                        publish_global(DomainEvent::SubagentFailed {
                            parent_session: background_parent_session,
                            task_id: outcome.task_id.clone(),
                            agent_id: outcome.agent_id.clone(),
                            error: error.clone(),
                        });
                        if let Some(ref tx) = background_progress {
                            let _ = tx
                                .send(AgentProgress::SubagentFailed {
                                    agent_id: outcome.agent_id,
                                    task_id: outcome.task_id,
                                    error,
                                })
                                .await;
                        }
                    }
                },
                Err(err) => {
                    let error = err.to_string();
                    publish_global(DomainEvent::SubagentFailed {
                        parent_session: background_parent_session,
                        task_id: background_task_id.clone(),
                        agent_id: background_agent_id.clone(),
                        error: error.clone(),
                    });
                    if let Some(ref tx) = background_progress {
                        let _ = tx
                            .send(AgentProgress::SubagentFailed {
                                agent_id: background_agent_id,
                                task_id: background_task_id,
                                error,
                            })
                            .await;
                    }
                }
            }
        });

        let payload = json!({
            "task_id": task_id,
            "agent_id": definition.id,
            "mode": "async",
            "worker_thread_id": worker_thread_id,
        });
        Ok(ToolResult::success(format!(
            "Accepted background sub-agent `{}`. Do not wait for it before answering the user.\n\n[async_subagent_ref]\n{}\n[/async_subagent_ref]",
            payload["agent_id"].as_str().unwrap_or("subagent"),
            serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string())
        )))
    }
}

fn add_background_contract(prompt: &str) -> String {
    format!(
        "[Background Contract]\n\
         Run this task without requiring attention from the parent or user. \
         Do not call ask_user_clarification. If required information is missing, \
         make the safest best-effort progress and record the limitation in your final output.\n\n\
         [Task]\n{prompt}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameters_schema_advertises_fire_and_forget_fields() {
        let tool = SpawnAsyncSubagentTool::new();
        let schema = tool.parameters_schema();
        let required = schema
            .get("required")
            .and_then(|v| v.as_array())
            .expect("required list");
        assert!(required.iter().any(|v| v.as_str() == Some("agent_id")));
        assert!(required.iter().any(|v| v.as_str() == Some("prompt")));

        let props = schema
            .get("properties")
            .and_then(|v| v.as_object())
            .expect("properties");
        for key in ["context", "model", "toolkit", "task_title"] {
            assert!(props.contains_key(key), "missing {key}");
        }
    }

    #[test]
    fn background_contract_forbids_user_attention() {
        let wrapped = add_background_contract("archive this fact");
        assert!(wrapped.contains("[Background Contract]"));
        assert!(wrapped.contains("Do not call ask_user_clarification"));
        assert!(wrapped.contains("[Task]\narchive this fact"));
    }

    #[tokio::test]
    async fn missing_agent_id_returns_error() {
        let tool = SpawnAsyncSubagentTool::new();
        let result = tool.execute(json!({ "prompt": "do work" })).await.unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("agent_id"));
    }

    #[tokio::test]
    async fn missing_prompt_returns_error() {
        let tool = SpawnAsyncSubagentTool::new();
        let result = tool
            .execute(json!({ "agent_id": "archivist" }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("prompt"));
    }
}
