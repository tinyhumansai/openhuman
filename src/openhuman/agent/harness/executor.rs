//! Orchestrator executor — the multi-agent run loop.
//!
//! When `orchestrator.enabled == true`, this replaces the default single-agent
//! tool loop with:
//!
//!   1. **Plan** — Spawn a Planner sub-agent to produce a `TaskDag`.
//!   2. **Execute** — Run DAG levels concurrently via `tokio::JoinSet`.
//!   3. **Review** — Orchestrator reviews each level's results.
//!   4. **Synthesise** — Final Orchestrator call to merge all results.

use super::archetypes::AgentArchetype;
use super::dag::TaskDag;
use super::types::{ReviewDecision, SubAgentResult, TaskStatus};
use crate::openhuman::config::{Config, OrchestratorConfig};
use crate::openhuman::memory::Memory;
use crate::openhuman::providers::{ChatMessage, ChatRequest, Provider};
use crate::openhuman::tools::Tool;
use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinSet;

/// Top-level entry point for orchestrated multi-agent execution.
///
/// Called from `Agent::turn()` when `config.orchestrator.enabled == true`.
pub async fn run_orchestrated(
    user_message: &str,
    config: &Config,
    provider: &dyn Provider,
    memory: Arc<dyn Memory>,
    _tools: &[Box<dyn Tool>],
    session_id: &str,
) -> Result<String> {
    let orch_config = &config.orchestrator;
    tracing::info!(
        "[orchestrator] starting orchestrated run for session={session_id}, max_dag_tasks={}",
        orch_config.max_dag_tasks
    );

    // ── 1. PLAN ──────────────────────────────────────────────────────────────
    let dag = plan_tasks(user_message, config, provider, memory.clone()).await?;
    tracing::debug!(
        "[orchestrator] planner produced {} task(s) for goal: {}",
        dag.len(),
        dag.root_goal
    );

    if dag.is_empty() {
        tracing::warn!(
            "[orchestrator] planner returned empty DAG, falling back to direct response"
        );
        return direct_response(user_message, provider, config).await;
    }

    // ── 2. EXECUTE ───────────────────────────────────────────────────────────
    let semaphore = Arc::new(tokio::sync::Semaphore::new(
        orch_config.max_concurrent_agents,
    ));
    let mut dag = dag;
    let levels = dag.execution_levels();
    let level_ids: Vec<Vec<String>> = levels
        .into_iter()
        .map(|lvl| lvl.into_iter().cloned().collect())
        .collect();

    for (level_idx, task_ids) in level_ids.iter().enumerate() {
        tracing::info!(
            "[orchestrator] executing level {}/{} with {} task(s)",
            level_idx + 1,
            level_ids.len(),
            task_ids.len()
        );

        let results = execute_level(
            &dag,
            task_ids,
            orch_config,
            config,
            provider,
            memory.clone(),
            session_id,
            semaphore.clone(),
        )
        .await;

        // Apply results to DAG nodes.
        for result in &results {
            if let Some(node) = dag.node_mut(&result.task_id) {
                node.status = if result.success {
                    TaskStatus::Completed
                } else {
                    TaskStatus::Failed
                };
                node.result = Some(result.clone());
            }
        }

        // ── 3. REVIEW ────────────────────────────────────────────────────────
        let decision = review_level(&dag, task_ids, provider, config).await?;
        match decision {
            ReviewDecision::Continue => {
                tracing::debug!(
                    "[orchestrator] level {} approved, continuing",
                    level_idx + 1
                );
            }
            ReviewDecision::Retry(retry_ids) => {
                tracing::info!(
                    "[orchestrator] retrying {} task(s) from level {}",
                    retry_ids.len(),
                    level_idx + 1
                );
                let retry_results = execute_level(
                    &dag,
                    &retry_ids,
                    orch_config,
                    config,
                    provider,
                    memory.clone(),
                    session_id,
                    semaphore.clone(),
                )
                .await;
                for result in retry_results {
                    if let Some(node) = dag.node_mut(&result.task_id) {
                        node.retry_count += 1;
                        node.status = if result.success {
                            TaskStatus::Completed
                        } else {
                            TaskStatus::Failed
                        };
                        node.result = Some(result);
                    }
                }
            }
            ReviewDecision::Abort(reason) => {
                tracing::warn!("[orchestrator] aborting DAG: {reason}");
                return Ok(format!(
                    "I had to stop the multi-step plan: {reason}\n\nHere's what was completed:\n{}",
                    summarise_completed(&dag)
                ));
            }
        }

        // After retries, check if any task in this level still failed.
        let still_failed: Vec<&str> = task_ids
            .iter()
            .filter(|id| {
                dag.node(id)
                    .is_some_and(|n| matches!(n.status, TaskStatus::Failed))
            })
            .map(|s| s.as_str())
            .collect();
        if !still_failed.is_empty() {
            tracing::warn!(
                "[orchestrator] level {} has {} task(s) still failed after retries, halting",
                level_idx + 1,
                still_failed.len()
            );
            return Ok(format!(
                "Plan halted: {} task(s) failed in level {}.\n\nCompleted so far:\n{}",
                still_failed.len(),
                level_idx + 1,
                summarise_completed(&dag)
            ));
        }
    }

    // ── 4. SYNTHESISE ────────────────────────────────────────────────────────
    synthesise_response(&dag, user_message, provider, config).await
}

// ─── Internal helpers ────────────────────────────────────────────────────────

/// Ask the Planner archetype to break the user goal into a TaskDag.
async fn plan_tasks(
    user_message: &str,
    config: &Config,
    provider: &dyn Provider,
    _memory: Arc<dyn Memory>,
) -> Result<TaskDag> {
    let model = resolve_model(AgentArchetype::Planner, &config.orchestrator);
    let temperature = resolve_temperature(AgentArchetype::Planner, &config.orchestrator);

    let system_prompt = format!(
        "You are the Planner agent. Break the user's goal into a task DAG.\n\
         Return ONLY valid JSON matching this schema:\n\
         ```json\n\
         {{\n\
           \"root_goal\": \"<user's goal>\",\n\
           \"nodes\": [\n\
             {{\n\
               \"id\": \"task-1\",\n\
               \"description\": \"what to do\",\n\
               \"archetype\": \"code_executor|skills_agent|researcher|critic|tool_maker\",\n\
               \"depends_on\": [],\n\
               \"acceptance_criteria\": \"how to verify success\"\n\
             }}\n\
           ]\n\
         }}\n\
         ```\n\
         Available archetypes: code_executor, skills_agent, tool_maker, researcher, critic.\n\
         Max {max} tasks. Use depends_on for ordering. Minimise task count.\n\
         If the goal is simple (single step), return exactly 1 node.",
        max = config.orchestrator.max_dag_tasks
    );

    let messages = vec![
        ChatMessage::system(&system_prompt),
        ChatMessage::user(user_message),
    ];

    let request = ChatRequest {
        messages: &messages,
        tools: None,
        system_prompt_cache_boundary: None,
    };

    let response = provider.chat(request, &model, temperature).await?;
    let text = response.text.as_deref().unwrap_or("").trim();

    // Extract JSON from potential markdown code fences.
    let json_str = extract_json_block(text);

    let dag: TaskDag =
        serde_json::from_str(json_str).context("failed to parse Planner DAG JSON")?;

    dag.validate()
        .map_err(|e| anyhow::anyhow!("invalid DAG: {e}"))?;

    Ok(dag)
}

/// Execute all tasks in a single DAG level concurrently.
///
/// Each task is dispatched through the unified
/// [`super::subagent_runner::run_subagent`] helper, which:
/// - Looks up the built-in [`super::definition::AgentDefinition`] for
///   the node's archetype.
/// - Resolves model + tool filtering + narrow prompt construction.
/// - Runs the sub-agent's inner tool-call loop using the parent's
///   provider via the [`super::fork_context::PARENT_CONTEXT`] task-local
///   that the orchestrator sets up earlier in
///   [`super::subagent_runner`].
///
/// Per-archetype overrides from
/// [`crate::openhuman::config::OrchestratorConfig::archetypes`] (model,
/// temperature, max_tool_iterations, timeout_secs, sandbox) are layered
/// on top of the built-in definition before dispatch.
#[allow(clippy::too_many_arguments)]
async fn execute_level(
    dag: &TaskDag,
    task_ids: &[String],
    orch_config: &OrchestratorConfig,
    _config: &Config,
    _provider: &dyn Provider,
    _memory: Arc<dyn Memory>,
    _session_id: &str,
    semaphore: Arc<tokio::sync::Semaphore>,
) -> Vec<SubAgentResult> {
    let mut join_set: JoinSet<SubAgentResult> = JoinSet::new();

    for task_id in task_ids {
        let Some(node) = dag.node(task_id) else {
            tracing::warn!("[orchestrator] task {task_id} not found in DAG, skipping");
            continue;
        };

        let archetype = node.archetype;
        let description = node.description.clone();
        let acceptance = node.acceptance_criteria.clone();
        let tid = task_id.clone();
        let semaphore_clone = semaphore.clone();
        let timeout = resolve_timeout(archetype, orch_config);

        // Collect context from completed dependencies.
        let dep_context: String = node
            .depends_on
            .iter()
            .filter_map(|dep_id| {
                dag.node(dep_id)
                    .and_then(|n| n.result.as_ref())
                    .map(|r| format!("## Result from {dep_id}\n{}\n", r.output))
            })
            .collect();

        // Build the sub-agent prompt with the task description and
        // acceptance criteria; dependency context (if any) flows
        // through the runner's `SubagentRunOptions::context` field.
        let prompt = format!("Task: {description}\n\nAcceptance criteria: {acceptance}");
        let context_blob = (!dep_context.is_empty()).then_some(dep_context);

        // Resolve the built-in definition for this archetype, layered
        // with any per-archetype config overrides.
        let mut definition = super::builtin_definitions::from_archetype(archetype);
        apply_archetype_overrides(&mut definition, archetype, orch_config);

        join_set.spawn(async move {
            let _permit = semaphore_clone
                .acquire_owned()
                .await
                .expect("semaphore closed");
            let start = Instant::now();

            let options = super::subagent_runner::SubagentRunOptions {
                skill_filter_override: None,
                category_filter_override: None,
                context: context_blob,
                task_id: Some(tid.clone()),
            };

            let outcome_fut = super::subagent_runner::run_subagent(&definition, &prompt, options);
            let outcome = match tokio::time::timeout(timeout, outcome_fut).await {
                Ok(Ok(out)) => out,
                Ok(Err(err)) => {
                    tracing::warn!(
                        task_id = %tid,
                        archetype = %archetype,
                        error = %err,
                        "[orchestrator] sub-agent failed"
                    );
                    return SubAgentResult {
                        task_id: tid,
                        success: false,
                        output: format!("sub-agent failed: {err}"),
                        artifacts: Vec::new(),
                        cost_microdollars: 0,
                        duration: start.elapsed(),
                    };
                }
                Err(_) => {
                    tracing::warn!(
                        task_id = %tid,
                        archetype = %archetype,
                        timeout_secs = timeout.as_secs(),
                        "[orchestrator] sub-agent timed out"
                    );
                    return SubAgentResult {
                        task_id: tid,
                        success: false,
                        output: format!("sub-agent timed out after {} seconds", timeout.as_secs()),
                        artifacts: Vec::new(),
                        cost_microdollars: 0,
                        duration: start.elapsed(),
                    };
                }
            };

            tracing::debug!(
                task_id = %outcome.task_id,
                archetype = %archetype,
                iterations = outcome.iterations,
                output_chars = outcome.output.chars().count(),
                "[orchestrator] sub-agent completed"
            );

            SubAgentResult {
                task_id: outcome.task_id,
                success: true,
                output: outcome.output,
                artifacts: Vec::new(),
                cost_microdollars: 0,
                duration: outcome.elapsed,
            }
        });
    }

    let mut results = Vec::new();
    while let Some(res) = join_set.join_next().await {
        match res {
            Ok(result) => results.push(result),
            Err(e) => {
                tracing::error!("[orchestrator] sub-agent task panicked: {e}");
            }
        }
    }
    results
}

/// Apply per-archetype config overrides on top of a built-in
/// [`super::definition::AgentDefinition`].
fn apply_archetype_overrides(
    definition: &mut super::definition::AgentDefinition,
    archetype: AgentArchetype,
    orch_config: &OrchestratorConfig,
) {
    let key = archetype.to_string();
    let Some(over) = orch_config.archetypes.get(&key) else {
        return;
    };
    if let Some(model) = over.model.as_ref() {
        // The override is a raw model name — store as Exact so the
        // runner uses it verbatim regardless of the parent's model.
        definition.model = super::definition::ModelSpec::Exact(model.clone());
    }
    if let Some(temperature) = over.temperature {
        definition.temperature = temperature;
    }
    if let Some(max_iter) = over.max_tool_iterations {
        definition.max_iterations = max_iter;
    }
    if let Some(secs) = over.timeout_secs {
        definition.timeout_secs = Some(secs);
    }
    if let Some(sb) = over.sandbox.as_deref() {
        definition.sandbox_mode = match sb {
            "sandboxed" => super::definition::SandboxMode::Sandboxed,
            "read_only" => super::definition::SandboxMode::ReadOnly,
            _ => super::definition::SandboxMode::None,
        };
    }
    if let Some(prompt_override) = over.system_prompt.as_ref() {
        definition.system_prompt = super::definition::PromptSource::Inline(prompt_override.clone());
    }
}

/// The Orchestrator reviews results from a completed level and decides next action.
async fn review_level(
    dag: &TaskDag,
    task_ids: &[String],
    _provider: &dyn Provider,
    config: &Config,
) -> Result<ReviewDecision> {
    let failed: Vec<&str> = task_ids
        .iter()
        .filter(|id| {
            dag.node(id)
                .is_some_and(|n| matches!(n.status, TaskStatus::Failed))
        })
        .map(|s| s.as_str())
        .collect();

    if failed.is_empty() {
        return Ok(ReviewDecision::Continue);
    }

    let retriable: Vec<String> = failed
        .iter()
        .filter(|&&id| {
            dag.node(id)
                .is_some_and(|n| n.retry_count < config.orchestrator.max_task_retries)
        })
        .map(|s| s.to_string())
        .collect();

    if retriable.is_empty() {
        return Ok(ReviewDecision::Abort(format!(
            "{} task(s) failed with no retries left",
            failed.len()
        )));
    }

    Ok(ReviewDecision::Retry(retriable))
}

/// Final Orchestrator call to synthesise all results into a user response.
async fn synthesise_response(
    dag: &TaskDag,
    user_message: &str,
    provider: &dyn Provider,
    config: &Config,
) -> Result<String> {
    let model = resolve_model(AgentArchetype::Orchestrator, &config.orchestrator);
    let temperature = resolve_temperature(AgentArchetype::Orchestrator, &config.orchestrator);

    let results_summary = summarise_completed(dag);

    let system_prompt = "You are the Orchestrator. Synthesise the sub-agent results into a \
                         coherent, helpful response for the user. Be concise and direct.";

    let user_msg = format!(
        "Original request: {user_message}\n\n\
         Sub-agent results:\n{results_summary}\n\n\
         Provide the final response."
    );

    let messages = vec![
        ChatMessage::system(system_prompt),
        ChatMessage::user(&user_msg),
    ];

    let request = ChatRequest {
        messages: &messages,
        tools: None,
        system_prompt_cache_boundary: None,
    };

    let response = provider.chat(request, &model, temperature).await?;
    Ok(response.text.unwrap_or_default())
}

/// Fallback: direct single-shot response when DAG is empty.
async fn direct_response(
    user_message: &str,
    provider: &dyn Provider,
    config: &Config,
) -> Result<String> {
    let model = config
        .default_model
        .as_deref()
        .unwrap_or(crate::openhuman::config::DEFAULT_MODEL);

    let response = provider
        .simple_chat(user_message, model, config.default_temperature)
        .await?;
    Ok(response)
}

/// Summarise all completed task results for the Orchestrator's synthesis step.
fn summarise_completed(dag: &TaskDag) -> String {
    dag.nodes
        .iter()
        .filter(|n| n.result.is_some())
        .map(|n| {
            let result = n.result.as_ref().unwrap();
            let status = if result.success { "OK" } else { "FAILED" };
            format!(
                "### {} [{}] ({})\n{}\n",
                n.id, status, n.archetype, result.output
            )
        })
        .collect()
}

/// Extract JSON from a string that may be wrapped in markdown code fences.
fn extract_json_block(text: &str) -> &str {
    // Try ```json ... ``` first.
    if let Some(start) = text.find("```json") {
        let content_start = start + 7;
        if let Some(end) = text[content_start..].find("```") {
            return text[content_start..content_start + end].trim();
        }
    }
    // Try ``` ... ```.
    if let Some(start) = text.find("```") {
        let content_start = start + 3;
        // Skip to next line if the fence has a language tag.
        let actual_start = text[content_start..]
            .find('\n')
            .map(|i| content_start + i + 1)
            .unwrap_or(content_start);
        if let Some(end) = text[actual_start..].find("```") {
            return text[actual_start..actual_start + end].trim();
        }
    }
    // Assume raw JSON.
    text.trim()
}

/// Resolve the model name for an archetype, respecting config overrides.
fn resolve_model(archetype: AgentArchetype, config: &OrchestratorConfig) -> String {
    let key = archetype.to_string();
    if let Some(ac) = config.archetypes.get(&key) {
        if let Some(ref model) = ac.model {
            return model.clone();
        }
    }
    format!("hint:{}", archetype.default_model_hint())
}

/// Resolve temperature for an archetype.
fn resolve_temperature(archetype: AgentArchetype, config: &OrchestratorConfig) -> f64 {
    config
        .archetypes
        .get(&archetype.to_string())
        .and_then(|ac| ac.temperature)
        .unwrap_or(0.4) // sub-agents default to lower temperature for precision
}

/// Resolve timeout for an archetype.
fn resolve_timeout(archetype: AgentArchetype, config: &OrchestratorConfig) -> Duration {
    let secs = config
        .archetypes
        .get(&archetype.to_string())
        .and_then(|ac| ac.timeout_secs)
        .unwrap_or_else(crate::openhuman::tool_timeout::tool_execution_timeout_secs);
    Duration::from_secs(secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::ArchetypeConfig;

    #[test]
    fn extract_json_from_fenced_block() {
        let input = "Here's the plan:\n```json\n{\"root_goal\": \"test\"}\n```\nDone.";
        assert_eq!(extract_json_block(input), "{\"root_goal\": \"test\"}");
    }

    #[test]
    fn extract_json_raw() {
        let input = "{\"root_goal\": \"test\"}";
        assert_eq!(extract_json_block(input), "{\"root_goal\": \"test\"}");
    }

    #[test]
    fn resolve_model_with_override() {
        let mut config = OrchestratorConfig::default();
        config.archetypes.insert(
            "code_executor".into(),
            ArchetypeConfig {
                model: Some("custom-model".into()),
                ..Default::default()
            },
        );
        assert_eq!(
            resolve_model(AgentArchetype::CodeExecutor, &config),
            "custom-model"
        );
    }

    #[test]
    fn resolve_model_default_hint() {
        let config = OrchestratorConfig::default();
        assert_eq!(
            resolve_model(AgentArchetype::CodeExecutor, &config),
            "hint:coding"
        );
        assert_eq!(
            resolve_model(AgentArchetype::Orchestrator, &config),
            "hint:reasoning"
        );
    }
}
