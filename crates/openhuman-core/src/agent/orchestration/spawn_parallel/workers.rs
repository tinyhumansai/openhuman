//! Running the staged workers on the module's [`run`]: the serial
//! shared-workspace-write fallback and bounded fan-out are tinyagents-owned.
//! This file keeps only the single-worker execution (`run_subagent` + post-run
//! worktree status snapshot) and the result projection.

use std::path::PathBuf;

use tinyagents_harness::CancellationToken;
use tinyagents_orchestration::parallel::{resolve_concurrency, run, Worker};

use crate::agent::subagent_host::{
    run_subagent_with_parent, SubagentRunOptions, SubagentRunStatus,
};

use super::types::{ParallelAgentResult, ParallelAgentStatus, SpawnParallelWorker};

pub(crate) async fn run_spawn_parallel_workers(
    prepared: Vec<Worker<SpawnParallelWorker>>,
    action_root: Option<PathBuf>,
    cancel: CancellationToken,
    run_context: crate::agent::tinyagents::host::OpenHumanRunContext,
    live_parent: &tinyagents_harness::context::RunContext<
        crate::agent::tinyagents::host::OpenHumanRunContext,
    >,
) -> tinyagents_harness::Result<Vec<ParallelAgentResult>> {
    let concurrency = resolve_concurrency(0, prepared.len());
    tracing::debug!(
        target: "orchestration",
        workers = prepared.len(),
        concurrency,
        "[orchestration] running spawn_parallel_agents on tinyagents-orchestration parallel::run"
    );
    run(
        prepared,
        concurrency,
        cancel,
        |_| run_context.child(),
        |worker, child_context| {
            run_one_parallel_task(worker, action_root.clone(), child_context, live_parent)
        },
    )
    .await
}

async fn run_one_parallel_task(
    worker: Worker<SpawnParallelWorker>,
    repo_root: Option<PathBuf>,
    run_context: crate::agent::tinyagents::host::OpenHumanRunContext,
    live_parent: &tinyagents_harness::context::RunContext<
        crate::agent::tinyagents::host::OpenHumanRunContext,
    >,
) -> ParallelAgentResult {
    let Worker {
        id: task_id,
        prompt,
        payload:
            SpawnParallelWorker {
                definition,
                task,
                lineage,
                worktree_path,
                workspace_descriptor,
            },
        ..
    } = worker;
    let started = std::time::Instant::now();
    tracing::debug!(
        task_id = %task_id,
        agent_id = %definition.id,
        context_chars = task.context.as_ref().map(|s| s.chars().count()).unwrap_or(0),
        prompt_chars = prompt.chars().count(),
        isolated = worktree_path.is_some(),
        "[spawn_parallel_agents] task_start"
    );
    let worktree_action_dir = worktree_path.clone().or_else(|| {
        workspace_descriptor
            .as_ref()
            .map(|descriptor| descriptor.root.clone())
    });
    let options = SubagentRunOptions {
        skill_filter_override: None,
        context: task.context.clone(),
        model_override: None,
        task_id: Some(task_id.clone()),
        thread_id: None,
        run_context,
        worker_thread_id: None,
        initial_history: None,
        checkpoint_dir: None,
        worktree_action_dir,
        workspace_descriptor,
        run_queue: None,
    };
    let run_result =
        run_subagent_with_parent(live_parent, definition.clone(), prompt.clone(), options).await;

    // After the worker finishes, snapshot the worktree's changed files +
    // dirty status so the parent can detect cross-worker overlaps and the UI
    // can surface diff/cleanup actions. Best-effort: a status error degrades
    // to "no changes recorded" rather than failing the task.
    let worktree_str = worktree_path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string());
    let (changed_files, dirty_status) = match (&worktree_path, &repo_root) {
        (Some(wt), Some(root)) => {
            match tinyagents_harness::workspace::git_worktree_status(root, wt) {
                Ok(st) => {
                    tracing::debug!(
                        task_id = %task_id,
                        worktree = %wt.display(),
                        is_dirty = st.is_dirty,
                        changed = st.changed_files.len(),
                        "[spawn_parallel_agents] worktree_post_run_status"
                    );
                    let files = st
                        .changed_files
                        .iter()
                        .map(|p| p.to_string_lossy().to_string())
                        .collect();
                    (files, Some(st.is_dirty))
                }
                Err(err) => {
                    tracing::warn!(
                        task_id = %task_id,
                        worktree = %wt.display(),
                        error = %err,
                        "[spawn_parallel_agents] worktree_status_failed"
                    );
                    (Vec::new(), None)
                }
            }
        }
        _ => (Vec::new(), None),
    };

    match run_result {
        Ok(outcome) => {
            let emit_lifecycle_effects = outcome.should_emit_lifecycle_effects();
            let (status, success, error, awaiting_question, checkpoint_path) = match &outcome.status
            {
                SubagentRunStatus::Completed => {
                    (ParallelAgentStatus::Completed, true, None, None, None)
                }
                SubagentRunStatus::AwaitingUser {
                    question,
                    checkpoint,
                    ..
                } => (
                    ParallelAgentStatus::AwaitingUser,
                    false,
                    None,
                    Some(question.clone()),
                    checkpoint
                        .as_ref()
                        .map(|path| path.to_string_lossy().to_string()),
                ),
                SubagentRunStatus::Incomplete { reason } => (
                    ParallelAgentStatus::Incomplete,
                    false,
                    Some(reason.clone()),
                    None,
                    None,
                ),
                SubagentRunStatus::Cancelled => (
                    ParallelAgentStatus::Cancelled,
                    false,
                    Some("parallel subagent was cancelled".into()),
                    None,
                    None,
                ),
            };
            tracing::debug!(
                task_id = %outcome.task_id,
                agent_id = %outcome.agent_id,
                elapsed_ms = outcome.elapsed.as_millis() as u64,
                iterations = outcome.iterations,
                output_chars = outcome.output.chars().count(),
                status = ?status,
                "[spawn_parallel_agents] task_outcome"
            );
            ParallelAgentResult {
                task_id: outcome.task_id,
                agent_id: outcome.agent_id,
                lineage,
                success,
                status,
                output: Some(outcome.output),
                error,
                awaiting_question,
                checkpoint_path,
                ownership: task.ownership,
                elapsed_ms: outcome.elapsed.as_millis() as u64,
                iterations: outcome.iterations as u32,
                stale_parent_reads: Vec::new(),
                worktree_path: worktree_str,
                changed_files,
                dirty_status,
                emit_lifecycle_effects,
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
                lineage,
                success: false,
                status: ParallelAgentStatus::Failed,
                output: None,
                error: Some(err.to_string()),
                awaiting_question: None,
                checkpoint_path: None,
                ownership: task.ownership,
                elapsed_ms: started.elapsed().as_millis() as u64,
                iterations: 0,
                stale_parent_reads: Vec::new(),
                worktree_path: worktree_str,
                changed_files,
                dirty_status,
                emit_lifecycle_effects: false,
            }
        }
    }
}
