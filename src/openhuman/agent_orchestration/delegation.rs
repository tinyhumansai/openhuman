//! Production wiring for the multi-stage sub-agent delegation graph (issue
//! #4249, Phase 3).
//!
//! [`tinyagents::delegation::run_delegation`](crate::openhuman::tinyagents::delegation::run_delegation)
//! is the durable plan→execute⇄review→finalize state machine, but it takes an
//! *injected* per-stage worker so its orchestration mechanics can be unit-tested
//! with a mock. This module supplies the **production** worker: every stage runs
//! through [`run_subagent`] (the same dispatch path `spawn_subagent` uses), and
//! the run is made durable/resumable by checkpointing the typed
//! [`DelegationState`] to the openhuman session DB through
//! [`SqlRunLedgerCheckpointer`].
//!
//! Layering: the delegation *graph* lives in the `tinyagents` adapter seam; this
//! production glue lives in `agent_orchestration`, which already depends on both
//! the seam and `subagent_runner` (so the seam stays free of orchestration deps).

use std::sync::Arc;

use crate::openhuman::agent::harness::definition::AgentDefinition;
use crate::openhuman::agent::harness::fork_context::{current_parent, with_parent_context};
use crate::openhuman::agent::harness::subagent_runner::{run_subagent, SubagentRunOptions};
use crate::openhuman::agent_orchestration::parent_context::build_root_parent;
use crate::openhuman::config::Config;
use crate::openhuman::tinyagents::delegation::{
    run_delegation, DelegationConfig, DelegationStage, DelegationStageOutput, DelegationState,
};
use crate::openhuman::tinyagents::SqlRunLedgerCheckpointer;
use tinyagents::graph::checkpoint::Checkpointer;
use tinyagents::CancellationToken;

const LOG_TARGET: &str = "agent_orchestration::delegation";

/// Run the durable plan→execute⇄review→finalize delegation graph for
/// `definition` against `task_prompt`, dispatching every stage to
/// [`run_subagent`] and checkpointing the typed state to the session DB so the
/// run is resumable. Returns the terminal [`DelegationState`] (its
/// `final_output` holds the synthesized answer).
///
/// Reuses the caller's enclosing agent turn when one is present (e.g. the
/// `delegate` tool). When none is — a controller/background caller — a root
/// parent is built so the nested `run_subagent` calls still resolve a provider,
/// tool registry, memory, and model (mirroring the workflow engine + team
/// runtime).
pub async fn run_subagent_delegation(
    config: Arc<Config>,
    definition: AgentDefinition,
    task_prompt: String,
    max_revisions: usize,
) -> Result<DelegationState, String> {
    let thread_id = format!("delegrun-{}", uuid::Uuid::new_v4());
    let checkpointer: Arc<dyn Checkpointer<DelegationState>> =
        Arc::new(SqlRunLedgerCheckpointer::new(config.clone()));

    tracing::info!(
        target: LOG_TARGET,
        agent_id = %definition.id,
        thread_id = %thread_id,
        max_revisions,
        "[delegation] starting durable sub-agent delegation"
    );

    let run = async move {
        // Re-entrant per-stage worker: clones its captures each call so the graph
        // node handler stays `Fn` while each stage dispatches a fresh sub-agent.
        let run_stage = move |stage: DelegationStage, state: DelegationState| {
            let definition = definition.clone();
            let task = task_prompt.clone();
            async move {
                let prompt = build_stage_prompt(stage, &task, &state);
                match run_subagent(&definition, &prompt, delegation_subagent_options()).await {
                    Ok(outcome) => {
                        let approved = matches!(stage, DelegationStage::Review)
                            && review_approves(&outcome.output);
                        Ok(DelegationStageOutput {
                            text: outcome.output,
                            approved,
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
            cancel: CancellationToken::new(),
        };
        run_delegation(delegation_config, run_stage).await
    };

    if current_parent().is_some() {
        run.await
    } else {
        let parent = build_root_parent(&config, "delegation_engine", "delegation", "delegation")
            .await
            .map_err(|e| format!("delegation: failed to build root parent: {e}"))?;
        with_parent_context(parent, run).await
    }
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
            let result = state
                .executions
                .last()
                .map(String::as_str)
                .unwrap_or("(no execution produced)");
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
fn delegation_subagent_options() -> SubagentRunOptions {
    SubagentRunOptions {
        skill_filter_override: None,
        toolkit_override: None,
        context: None,
        model_override: None,
        task_id: None,
        worker_thread_id: None,
        initial_history: None,
        checkpoint_dir: None,
        worktree_action_dir: None,
        run_queue: None,
    }
}
