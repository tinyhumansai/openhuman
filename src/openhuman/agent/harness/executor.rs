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
        tracing::warn!("[orchestrator] planner returned empty DAG, falling back to direct response");
        return direct_response(user_message, provider, config).await;
    }

    // ── 2. EXECUTE ───────────────────────────────────────────────────────────
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
                tracing::debug!("[orchestrator] level {} approved, continuing", level_idx + 1);
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
    let text = response
        .text
        .as_deref()
        .unwrap_or("")
        .trim();

    // Extract JSON from potential markdown code fences.
    let json_str = extract_json_block(text);

    let dag: TaskDag = serde_json::from_str(json_str)
        .context("failed to parse Planner DAG JSON")?;

    dag.validate().map_err(|e| anyhow::anyhow!("invalid DAG: {e}"))?;

    Ok(dag)
}

/// Execute all tasks in a single DAG level concurrently.
async fn execute_level(
    dag: &TaskDag,
    task_ids: &[String],
    orch_config: &OrchestratorConfig,
    _config: &Config,
    _provider: &dyn Provider,
    _memory: Arc<dyn Memory>,
    session_id: &str,
) -> Vec<SubAgentResult> {
    let mut join_set: JoinSet<SubAgentResult> = JoinSet::new();
    let _semaphore = Arc::new(tokio::sync::Semaphore::new(
        orch_config.max_concurrent_agents,
    ));

    for task_id in task_ids {
        let Some(node) = dag.node(task_id) else {
            tracing::warn!("[orchestrator] task {task_id} not found in DAG, skipping");
            continue;
        };

        let archetype = node.archetype;
        let description = node.description.clone();
        let acceptance = node.acceptance_criteria.clone();
        let tid = task_id.clone();
        let _sid = session_id.to_string();
        let model = resolve_model(archetype, orch_config);
        let _temperature = resolve_temperature(archetype, orch_config);
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

        let _prompt = if dep_context.is_empty() {
            format!(
                "Task: {description}\n\nAcceptance criteria: {acceptance}"
            )
        } else {
            format!(
                "Context from prior tasks:\n{dep_context}\n\
                 Task: {description}\n\nAcceptance criteria: {acceptance}"
            )
        };

        // Each sub-agent runs as a single-shot provider call for now.
        // Phase 3 will upgrade this to full tool-loop sub-agents.
        let _system_prompt = format!(
            "You are the {archetype} agent. Complete the assigned task precisely.\n\
             Do not deviate from the task description. Be concise."
        );
        let model_clone = model.clone();
        let _timeout = timeout;

        join_set.spawn(async move {
            let start = Instant::now();

            // For now, sub-agents use a simple prompt (no tool loop).
            // This will be upgraded when archetype-specific tool subsets are wired.
            let result_text = format!(
                "[placeholder: {archetype} sub-agent would execute here]\n\
                 Task: {description}\nModel: {model_clone}\nTimeout: {timeout:?}"
            );

            tracing::debug!(
                "[orchestrator] sub-agent {archetype} completed task {tid} in {:?}",
                start.elapsed()
            );

            SubAgentResult {
                task_id: tid,
                success: true,
                output: result_text,
                artifacts: Vec::new(),
                cost_microdollars: 0,
                duration: start.elapsed(),
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
        .unwrap_or(120);
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
