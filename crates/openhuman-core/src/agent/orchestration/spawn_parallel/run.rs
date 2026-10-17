//! Host-side execution of an already-decoded `spawn_parallel_agents` batch.
//!
//! The fixed phase graph formerly here duplicated the graph crate's bounded
//! fanout facility while capturing OpenHuman-only policy, progress, and
//! execution adapters in every node. The tool now decodes its JSON contract;
//! this module runs the host stages directly and delegates concurrent work to
//! [`tinyagents_graph::parallel::map_reduce`] in `workers`.

use std::path::PathBuf;

use tinyagents_harness::CancellationToken;
use tinytools::WorkspaceDescriptor;

use crate::agent::harness::host_definition::AgentDefinitionRegistry;

use super::collect::{
    collect_spawn_parallel_results, project_spawn_parallel_result, SpawnParallelGraphOutcome,
};
use super::dispatch::stage_spawn_parallel_workers_from_defs;
use super::staging::snapshot_agent_definitions;
use super::types::ParallelAgentTask;
use super::workers::run_spawn_parallel_workers;

pub(crate) async fn run_spawn_parallel_tasks_with_cancellation_and_workspace(
    tasks: Vec<ParallelAgentTask>,
    cancel: CancellationToken,
    parent_workspace_descriptor: Option<WorkspaceDescriptor>,
    run_context: crate::agent::tinyagents::host::OpenHumanRunContext,
    live_parent: &tinyagents_harness::context::RunContext<
        crate::agent::tinyagents::host::OpenHumanRunContext,
    >,
    parent_call_id: Option<String>,
) -> Result<SpawnParallelGraphOutcome, String> {
    let parent = match run_context.parent.clone() {
        Some(parent) => parent,
        None => {
            tracing::debug!("[spawn_parallel_agents] rejected_outside_agent_turn");
            return Ok(SpawnParallelGraphOutcome::Rejected(
                "spawn_parallel_agents called outside of an agent turn".to_string(),
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
    let registry = match AgentDefinitionRegistry::current() {
        Some(registry) => registry,
        None => {
            tracing::debug!("[spawn_parallel_agents] registry_unavailable");
            return Ok(SpawnParallelGraphOutcome::Rejected(
                "spawn_parallel_agents: AgentDefinitionRegistry has not been initialised"
                    .to_string(),
            ));
        }
    };

    let parent_session = parent.session_id.clone();
    let progress_sink = parent.on_progress.clone();
    let action_root =
        resolve_spawn_parallel_action_root(parent_workspace_descriptor.as_ref()).await;
    let definitions = snapshot_agent_definitions(&registry);
    if cancel.is_cancelled() {
        return Ok(SpawnParallelGraphOutcome::Cancelled(
            "spawn_parallel_agents cancelled at validate".to_string(),
        ));
    }
    if tasks.len() > max_parallel {
        return Ok(SpawnParallelGraphOutcome::Rejected(format!(
            "spawn_parallel_agents received {} tasks but max_parallel_tools is {}",
            tasks.len(),
            max_parallel
        )));
    }
    if cancel.is_cancelled() {
        return Ok(SpawnParallelGraphOutcome::Cancelled(
            "spawn_parallel_agents cancelled at dispatch".to_string(),
        ));
    }
    let (prepared, immediate_results) = stage_spawn_parallel_workers_from_defs(
        &parent_session,
        progress_sink.as_ref(),
        tasks,
        &definitions,
        &parent,
        action_root.as_deref(),
        parent_workspace_descriptor.as_ref(),
        parent_call_id.as_deref(),
    )
    .await;
    if cancel.is_cancelled() {
        return Ok(SpawnParallelGraphOutcome::Cancelled(
            "spawn_parallel_agents cancelled at worker".to_string(),
        ));
    }
    let fanned = match run_spawn_parallel_workers(
        prepared,
        action_root,
        cancel.clone(),
        run_context,
        live_parent,
    )
    .await
    {
        Ok(fanned) => fanned,
        Err(tinyagents_harness::TinyAgentsError::Cancelled) => {
            return Ok(SpawnParallelGraphOutcome::Cancelled(
                "spawn_parallel_agents cancelled at worker".to_string(),
            ));
        }
        Err(err) => return Err(err.to_string()),
    };
    if cancel.is_cancelled() {
        return Ok(SpawnParallelGraphOutcome::Cancelled(
            "spawn_parallel_agents cancelled at collect".to_string(),
        ));
    }
    let mut results = immediate_results;
    for result in fanned {
        project_spawn_parallel_result(&parent_session, progress_sink.as_ref(), &result).await;
        results.push(result);
    }
    if cancel.is_cancelled() {
        return Ok(SpawnParallelGraphOutcome::Cancelled(
            "spawn_parallel_agents cancelled at finalize".to_string(),
        ));
    }
    let outcome = SpawnParallelGraphOutcome::Collected(collect_spawn_parallel_results(
        &parent_session,
        results,
    ));
    match &outcome {
        SpawnParallelGraphOutcome::Collected(collected) => {
            tracing::debug!(
                parent_session = %parent_session,
                total = collected.total(),
                succeeded = collected.succeeded(),
                failed = collected.failures,
                overlaps = collected.overlap_warnings.len(),
                "[spawn_parallel_agents] execute exit"
            );
        }
        SpawnParallelGraphOutcome::Rejected(message) => {
            tracing::debug!(
                parent_session = %parent_session,
                error = %message,
                "[spawn_parallel_agents] rejected_by_graph_validate"
            );
        }
        SpawnParallelGraphOutcome::Cancelled(message) => {
            tracing::debug!(
                parent_session = %parent_session,
                message = %message,
                "[spawn_parallel_agents] cancelled_by_graph"
            );
        }
    }
    Ok(outcome)
}

/// Resolve the agent sandbox root once for the graph run.
///
/// This is `Config.action_dir` (the user's project repo the coding agent edits),
/// NOT OpenHuman's own tree. It is only consulted when a worker asks for
/// git-worktree isolation; failures preserve the previous `None` fallback.
async fn resolve_spawn_parallel_action_root(
    parent_workspace_descriptor: Option<&WorkspaceDescriptor>,
) -> Option<PathBuf> {
    if let Some(descriptor) = parent_workspace_descriptor {
        tracing::debug!(
            action_root = %descriptor.root.display(),
            policy_id = %descriptor.policy_id,
            "[spawn_parallel_agents] using ToolExecutionContext workspace root for graph"
        );
        return Some(descriptor.root.clone());
    }
    match crate::config::Config::load_or_init().await {
        Ok(config) => {
            tracing::debug!(
                action_root = %config.action_dir.display(),
                "[spawn_parallel_agents] resolved action root for graph"
            );
            Some(config.action_dir.clone())
        }
        Err(err) => {
            tracing::debug!(
                error = %err,
                "[spawn_parallel_agents] config load failed; worktree isolation will use missing-root fallback"
            );
            None
        }
    }
}
