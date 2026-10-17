//! Production wiring for the multi-stage sub-agent delegation graph (issue
//! #4249, Phase 3).
//!
//! [`tinyagents_graph::delegation::run_delegation`](tinyagents_graph::delegation::run_delegation)
//! is the durable plan→execute⇄review→finalize state machine, but it takes an
//! *injected* per-stage worker so its orchestration mechanics can be unit-tested
//! with a mock. This module supplies the **production** worker: every stage runs
//! through [`run_subagent`] (the same dispatch path `spawn_subagent` uses), and
//! the run is made durable/resumable by checkpointing the typed
//! [`DelegationState`] through the crate
//! [`DriverCheckpointer`](tinyagents_graph::checkpoint::DriverCheckpointer) over
//! the storage backend.
//!
//! Layering: the delegation *graph* lives in the `tinyagents` adapter seam; this
//! production glue lives in `agent_orchestration`, which already depends on both
//! the seam and `subagent_host` (so the seam stays free of orchestration deps).

use std::sync::Arc;

use crate::agent::harness::host_definition::AgentDefinition;
use crate::agent::progress::AgentProgress;
use crate::agent::subagent_host::{
    run_subagent_with_parent, SubagentRunOptions, SubagentRunStatus,
};
use crate::agent::tinyagents::host::delegation::run_or_resume_with_tracing;
use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::config::Config;
use tinyagents_graph::checkpoint::Checkpointer;
use tinyagents_graph::delegation::{
    DelegationConfig, DelegationStage, DelegationStageOutput, DelegationState,
};
use tinyagents_harness::context::{RunConfig, RunContext};
use tinytools::WorkspaceDescriptor;

const LOG_TARGET: &str = "agent_orchestration::delegation";

/// Derives one delegation stage's run context and host data from the stage
/// parent. The host carrier is pointed at the stage run's own linked child
/// token, so a graph cancel still reaches the stage through the parent while
/// cancelling the stage leaves the graph (and its siblings) running.
pub(crate) fn stage_run_contexts(
    stage_parent: &RunContext<OpenHumanRunContext>,
    workspace: Option<WorkspaceDescriptor>,
) -> tinyagents_harness::Result<(RunContext<OpenHumanRunContext>, OpenHumanRunContext)> {
    let mut stage_context = stage_parent.data.child();
    stage_context.workspace = workspace.or(stage_context.workspace);
    let mut stage_run = stage_parent.child(
        RunConfig::new(format!("delegation-stage-{}", uuid::Uuid::new_v4())),
        stage_context.clone(),
    )?;
    stage_context.cancellation = stage_run.cancellation.clone();
    stage_run.data.cancellation = stage_run.cancellation.clone();
    Ok((stage_run, stage_context))
}

/// Typed live entrypoint for the durable delegation graph.
///
/// The graph does create a durable graph id for its checkpoints, but each
/// stage retains the caller's cancellation, origin, dispatch, progress,
/// conversation thread, and workspace through the supplied carrier.
pub(crate) async fn run_subagent_delegation_with_parent_context(
    config: Arc<Config>,
    definition: AgentDefinition,
    task_prompt: String,
    max_revisions: usize,
    parent_workspace_descriptor: Option<WorkspaceDescriptor>,
    live_parent: Arc<RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>>,
) -> Result<DelegationState, String> {
    let thread_id = format!("delegrun-{}", uuid::Uuid::new_v4());
    // The graph and every stage share this cancellation token. A stage gets an
    // owned host carrier before it enters `run_subagent`, rather than making a
    // fresh context after the graph has spawned work.
    let graph_cancellation = live_parent.cancellation.clone();
    // Durable graph checkpoints live in the storage backend, in the acting
    // agent's scope (`DriverCheckpointer`).
    let checkpointer = open_delegation_checkpointer(&config)?;

    tracing::info!(
        target: LOG_TARGET,
        agent_id = %definition.id,
        thread_id = %thread_id,
        max_revisions,
        "[delegation] starting durable sub-agent delegation"
    );
    if let Some(descriptor) = parent_workspace_descriptor.as_ref() {
        tracing::debug!(
            target: LOG_TARGET,
            agent_id = %definition.id,
            thread_id = %thread_id,
            workspace_root = %descriptor.root.display(),
            policy_id = %descriptor.policy_id,
            "[delegation] using ToolExecutionContext workspace root"
        );
    }

    let run = async move {
        // Re-entrant per-stage worker: clones its captures each call so the graph
        // node handler stays `Fn` while each stage dispatches a fresh sub-agent.
        let parent_workspace_descriptor = parent_workspace_descriptor.clone();
        let stage_parent = live_parent.clone();
        let run_stage = move |stage: DelegationStage, state: DelegationState| {
            let definition = definition.clone();
            let task = task_prompt.clone();
            let workspace_descriptor = parent_workspace_descriptor.clone();
            let stage_parent = stage_parent.clone();
            async move {
                let prompt = build_stage_prompt(stage, &task, &state);
                let (stage_parent, stage_context) =
                    stage_run_contexts(&stage_parent, workspace_descriptor.clone())
                        .map_err(|error| format!("delegation stage context: {error}"))?;
                match run_subagent_with_parent(
                    &stage_parent,
                    definition,
                    prompt.clone(),
                    delegation_subagent_options(workspace_descriptor, stage_context),
                )
                .await
                {
                    Ok(outcome) => {
                        let should_emit_lifecycle_effects = outcome.should_emit_lifecycle_effects();
                        match outcome.status {
                            SubagentRunStatus::Completed => {}
                            SubagentRunStatus::AwaitingUser {
                                question,
                                checkpoint,
                                ..
                            } => {
                                // A durable delegation graph cannot safely advance
                                // plan/review state while one of its stages is
                                // paused. Surface the exact durable child handle so
                                // the caller can continue that stage instead of
                                // silently stranding the checkpoint behind a generic
                                // graph error.
                                if should_emit_lifecycle_effects {
                                    let parent_session = stage_parent
                                        .data
                                        .parent
                                        .as_ref()
                                        .map(|parent| parent.session_id.clone())
                                        .unwrap_or_else(|| "standalone".to_string());
                                    crate::agent::orchestration::subagent_events::publish_subagent_awaiting_user(
                                        parent_session,
                                        outcome.task_id.clone(),
                                        outcome.agent_id.clone(),
                                        question.clone(),
                                    );
                                    if let Some(progress) = stage_parent.data.progress.clone() {
                                        let _ = progress
                                            .send(AgentProgress::SubagentAwaitingUser {
                                                agent_id: outcome.agent_id.clone(),
                                                task_id: outcome.task_id.clone(),
                                                question: question.clone(),
                                                worker_thread_id: None,
                                                checkpoint_path: checkpoint
                                                    .as_ref()
                                                    .map(|path| path.to_string_lossy().to_string()),
                                            })
                                            .await;
                                    }
                                }
                                return Err(format!(
                                    "[SUBAGENT_AWAITING_USER]\nagent_id: {}\ntask_id: {}\nquestion: {}\ncheckpointed: {}\n[/SUBAGENT_AWAITING_USER]\n\
                                     delegation stage {stage:?} is awaiting user input; relay the answer with continue_subagent using this task_id.",
                                    outcome.agent_id,
                                    outcome.task_id,
                                    serde_json::to_string(&question).unwrap_or_else(|_| {
                                        "\"<unserializable question>\"".to_string()
                                    }),
                                    checkpoint.is_some(),
                                ));
                            }
                            SubagentRunStatus::Incomplete { reason } => {
                                return Err(format!(
                                    "delegation stage {stage:?} stopped incomplete: {reason}"
                                ));
                            }
                            SubagentRunStatus::Cancelled => {
                                return Err(format!("delegation stage {stage:?} was cancelled"));
                            }
                        }
                        let approved = matches!(stage, DelegationStage::Review)
                            && review_approves(&outcome.output);
                        Ok(DelegationStageOutput {
                            text: outcome.output,
                            approved,
                            // Persist the exact prompt this stage sent so the
                            // execute step's `StepRecord` carries its provenance
                            // (read only for the execute stage; #3884).
                            prompt: Some(prompt),
                        })
                    }
                    Err(e) => Err(format!("delegation stage {stage:?} failed: {e}")),
                }
            }
        };

        let delegation_config = DelegationConfig {
            max_revisions,
            checkpointer: Some(checkpointer),
            thread_id: Some(thread_id),
            cancel: graph_cancellation,
            // Automated (non-human-gated) delegation: the reviewer stage decides
            // approve/revise on its own. The durable human-approval interrupt
            // (see `tinyagents_graph::delegation::run_delegation_durable`) is opt-in and
            // stays off here until a human-review delegation surface wires it.
            ..DelegationConfig::default()
        };
        // Resume-aware entry (#3884): with today's fresh-per-run `thread_id` this
        // is always a fresh run; reusing a stable `thread_id` resumes from the
        // last checkpoint boundary instead of restarting.
        run_or_resume_with_tracing(delegation_config, run_stage)
            .await
            .map(|outcome| outcome.state)
    };

    run.await
}

/// Per-stage prompt builder: each stage sees the task plus the accumulated state
/// (the plan for `execute`, the latest result for `review`, and the latest
/// reviewer feedback when re-executing after a revision request).
fn build_stage_prompt(stage: DelegationStage, task: &str, state: &DelegationState) -> String {
    match stage {
        DelegationStage::Plan => format!(
            "Produce a short, concrete, numbered plan to accomplish the task below. \
             Reply with the plan only.\n\n[Task]\n{task}"
        ),
        DelegationStage::Execute => {
            let plan = state.plan.as_deref().unwrap_or("(no plan produced)");
            let feedback = state
                .reviews
                .last()
                .map(|r| format!("\n\n[Reviewer feedback to address]\n{r}"))
                .unwrap_or_default();
            format!(
                "Carry out the plan below for the task and return the completed result.\n\n\
                 [Task]\n{task}\n\n[Plan]\n{plan}{feedback}"
            )
        }
        DelegationStage::Review => {
            let result = state.last_result().unwrap_or("(no execution produced)");
            format!(
                "Review the result below against the task. If it fully and correctly \
                 accomplishes the task, reply with `APPROVE` on the first line. Otherwise \
                 reply with `REVISE` on the first line followed by specific, actionable \
                 feedback.\n\n[Task]\n{task}\n\n[Result]\n{result}"
            )
        }
    }
}

/// A review stage approves when its first line begins with `APPROVE`.
fn review_approves(output: &str) -> bool {
    output
        .lines()
        .next()
        .map(|l| l.trim().to_ascii_uppercase())
        .map(|l| l.starts_with("APPROVE"))
        .unwrap_or(false)
}

/// Default sub-agent options for a delegation stage — a fresh UUID task id per
/// call (so retries/revisions don't collide), everything else inherited.
fn delegation_subagent_options(
    workspace_descriptor: Option<WorkspaceDescriptor>,
    run_context: crate::agent::tinyagents::host::OpenHumanRunContext,
) -> SubagentRunOptions {
    let worktree_action_dir = workspace_descriptor
        .as_ref()
        .map(|descriptor| descriptor.root.clone());
    SubagentRunOptions {
        skill_filter_override: None,
        context: None,
        model_override: None,
        task_id: None,
        thread_id: run_context.thread_id.clone(),
        run_context,
        worker_thread_id: None,
        initial_history: None,
        checkpoint_dir: None,
        worktree_action_dir,
        workspace_descriptor,
        run_queue: None,
    }
}

/// Open the delegation graph's durable checkpointer: the acting agent's scope
/// on the storage backend ([`crate::storage::local::open`]).
pub fn open_delegation_checkpointer(
    config: &Config,
) -> Result<Arc<dyn Checkpointer<DelegationState>>, String> {
    let scoped = crate::storage::local::open("delegation", &config.workspace_dir)
        .and_then(|opened| opened.scoped())
        .map_err(|e| format!("open the storage for graph checkpoints: {e:#}"))?;
    Ok(Arc::new(
        tinyagents_graph::checkpoint::DriverCheckpointer::<DelegationState>::with_namespace(
            Arc::clone(scoped.documents()),
            "delegation",
        ),
    ))
}

#[cfg(test)]
#[path = "delegation_tests.rs"]
mod tests;
