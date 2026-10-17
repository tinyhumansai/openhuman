//! Public entry points (`start`/`stop`/`resume`) and the spawned engine loop
//! that drives a run's phase DAG to completion.

use crate::core::runtime::DispatchCtx;
use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::sync::Arc;
use tinyagents_orchestration::workflow::{reset_running_phases, WorkflowEngine};

use crate::agent::harness::ParentExecutionContext;
use crate::agent::orchestration::parent_context::with_root_parent;
use crate::config::Config;
use tinyagents_session::run_ledger::{WorkflowRun, WorkflowRunStatus, WorkflowRunUpsert};

use super::super::ops::{
    compare_and_swap_workflow_run_lifecycle, definition_by_id, get_workflow_run, workflow_store,
};
use super::cancel::{
    cancel_signal_if_current, clear_cancel_signal, is_current_cancel_signal, lookup_cancel_signal,
    register_cancel_signal, replace_cancel_signal, WorkflowCancelSignal,
};
use super::LOG_TARGET;
use tinyagents_orchestration::workflow::WorkflowDefinition;

/// Start a new workflow run and return immediately.
///
/// Resolves `definition_id` to a builtin [`WorkflowDefinition`], creates a
/// `Running` ledger row with `phase_states` initialised to one `pending` entry
/// per phase, persists it, then `tokio::spawn`s the engine loop. The returned
/// [`WorkflowRun`] is the freshly-created row (status `Running`); callers poll
/// `workflow_run_get` to observe progress.
pub async fn start_workflow_run(
    ctx: &DispatchCtx,
    config: &Config,
    definition_id: &str,
    input: Value,
    parent_thread_id: Option<String>,
) -> Result<WorkflowRun> {
    log::debug!(
        target: LOG_TARGET,
        "[workflow_run_engine] start.entry definition={definition_id} parent_thread={parent_thread_id:?}"
    );
    let definition = definition_by_id(definition_id)
        .ok_or_else(|| anyhow!("unknown workflow definition: {definition_id}"))?;
    let safety_tier = super::super::host::admit_workflow(&definition)?;

    let run_id = format!("wfrun-{}", uuid::Uuid::new_v4());
    let initial_engine = WorkflowEngine::new(
        workflow_store(config)?,
        Arc::new(super::super::host::OpenHumanWorkflowExecutor::new(
            &run_id,
            None,
            safety_tier,
            None,
        )),
    );
    let run = initial_engine
        .initialise(run_id.clone(), &definition, input.clone(), parent_thread_id)
        .context("persist initial workflow run")?;

    let cancel = register_cancel_signal(&run_id);

    // Spawn the engine loop. Clone what the task needs (the engine reloads
    // config inside the task so it can build a real Agent without holding a
    // borrow across the spawn boundary).
    let task_run_id = run_id.clone();
    // Task-locals don't cross `tokio::spawn`, so capture the starting turn's
    // origin here and re-scope it around the engine loop — the phases spawn
    // sub-agents whose tool calls the approval gate judges by that label.
    // Inherit-only: `None` leaves the loop unlabelled and failing closed.
    let inherited_origin = crate::agent::turn_origin::capture();
    let task_ctx = ctx.clone();
    ctx.spawn(async move {
        match crate::config::ops::load_current_or_init(&task_ctx).await {
            Ok(task_config) => {
                crate::agent::turn_origin::with_inherited_origin(
                    inherited_origin,
                    run_engine_loop(
                        &task_ctx,
                        &task_config,
                        &task_run_id,
                        definition,
                        cancel,
                        None,
                    ),
                )
                .await;
            }
            Err(err) => {
                log::error!(
                    target: LOG_TARGET,
                    "[workflow_run_engine] start.config_load_failed run={task_run_id} err={err}"
                );
                clear_cancel_signal(&task_run_id, &cancel);
            }
        }
    });

    log::debug!(
        target: LOG_TARGET,
        "[workflow_run_engine] start.spawned run={run_id} phases={}",
        run.phase_states.as_object().map(|m| m.len()).unwrap_or(0)
    );
    Ok(run)
}

/// Signal a running workflow to stop after its current phase.
///
/// Flips the run's cancellation flag (checked by the loop between phases) and
/// eagerly marks the persisted row `Interrupted` so a poller sees the intent
/// immediately even while the in-flight phase drains. Idempotent: stopping a
/// terminal or unknown run is a no-op that returns the current row.
pub async fn stop_workflow_run(config: &Config, id: &str) -> Result<Option<WorkflowRun>> {
    log::debug!(target: LOG_TARGET, "[workflow_run_engine] stop.entry run={id}");
    let Some(run) = get_workflow_run(config, id)? else {
        log::debug!(target: LOG_TARGET, "[workflow_run_engine] stop.unknown run={id}");
        return Ok(None);
    };

    if matches!(
        run.status,
        WorkflowRunStatus::Completed | WorkflowRunStatus::Failed | WorkflowRunStatus::Cancelled
    ) {
        log::debug!(
            target: LOG_TARGET,
            "[workflow_run_engine] stop.already_terminal run={id} status={}",
            run.status.as_str()
        );
        return Ok(Some(run));
    }

    // Capture the exact live generation before fencing its durable row. If a
    // concurrent resume wins instead, this stop's CAS fails and it must not
    // cancel that successor's fresh token.
    let cancel = lookup_cancel_signal(id);

    let mut phase_states = run.phase_states.clone();
    reset_running_phases(
        &mut phase_states,
        "workflow interrupted by host; phase will retry on resume",
    );
    let updated = compare_and_swap_workflow_run_lifecycle(
        config,
        WorkflowRunUpsert {
            id: run.id.clone(),
            definition_id: run.definition_id.clone(),
            parent_thread_id: run.parent_thread_id.clone(),
            input: run.input.clone(),
            phase_states,
            child_run_ids: run.child_run_ids.clone(),
            status: WorkflowRunStatus::Interrupted,
            summary: run.summary.clone(),
            started_at: Some(run.started_at),
            completed_at: None,
        },
        run.revision,
    )
    .context("persist workflow run interrupt")?;
    let updated = updated.ok_or_else(|| {
        anyhow!("workflow run {id} changed while stop was being applied; reload and retry")
    })?;
    if let Some(cancel) = cancel.as_ref() {
        let _ = cancel_signal_if_current(id, cancel);
    }

    log::debug!(target: LOG_TARGET, "[workflow_run_engine] stop.marked_interrupted run={id}");
    Ok(Some(updated))
}

/// Resume an interrupted (or otherwise incomplete) workflow run.
///
/// Reloads the run, clears any stale cancellation flag, flips the row back to
/// `Running`, and spawns a fresh engine loop. Phases already `completed` in
/// `phase_states` are skipped; the loop continues from the first incomplete
/// phase whose dependencies are satisfied. Returns the run row (now `Running`),
/// or an error if the run is unknown / already terminal-complete / its
/// definition no longer exists.
pub async fn resume_workflow_run(
    ctx: &DispatchCtx,
    config: &Config,
    id: &str,
) -> Result<WorkflowRun> {
    log::debug!(target: LOG_TARGET, "[workflow_run_engine] resume.entry run={id}");
    let run = get_workflow_run(config, id)?.ok_or_else(|| anyhow!("unknown workflow run: {id}"))?;

    if matches!(run.status, WorkflowRunStatus::Completed) {
        return Err(anyhow!("workflow run {id} is already completed"));
    }

    let definition = definition_by_id(&run.definition_id)
        .ok_or_else(|| anyhow!("definition {} no longer exists", run.definition_id))?;
    let _safety_tier = super::super::host::admit_workflow(&definition)?;

    let resumed = compare_and_swap_workflow_run_lifecycle(
        config,
        WorkflowRunUpsert {
            id: run.id.clone(),
            definition_id: run.definition_id.clone(),
            parent_thread_id: run.parent_thread_id.clone(),
            input: run.input.clone(),
            phase_states: run.phase_states.clone(),
            child_run_ids: run.child_run_ids.clone(),
            status: WorkflowRunStatus::Running,
            summary: run.summary.clone(),
            started_at: Some(run.started_at),
            completed_at: None,
        },
        run.revision,
    )
    .context("persist workflow run resume")?;
    let resumed = resumed.ok_or_else(|| {
        anyhow!("workflow run {id} changed while resume was being applied; reload and retry")
    })?;

    // Only replace the process-local signal after the durable hand-off won.
    // Otherwise a losing resumer could erase the live driver's stop request.
    let cancel = replace_cancel_signal(id);

    let task_run_id = id.to_string();
    // Same inherit-only origin propagation as `start_workflow_run`: the resumed
    // loop runs on a fresh task, which would otherwise drop the caller's label.
    let inherited_origin = crate::agent::turn_origin::capture();
    let task_ctx = ctx.clone();
    ctx.spawn(async move {
        match crate::config::ops::load_current_or_init(&task_ctx).await {
            Ok(task_config) => {
                crate::agent::turn_origin::with_inherited_origin(
                    inherited_origin,
                    run_engine_loop(
                        &task_ctx,
                        &task_config,
                        &task_run_id,
                        definition,
                        cancel,
                        None,
                    ),
                )
                .await;
            }
            Err(err) => {
                log::error!(
                    target: LOG_TARGET,
                    "[workflow_run_engine] resume.config_load_failed run={task_run_id} err={err}"
                );
                clear_cancel_signal(&task_run_id, &cancel);
            }
        }
    });

    log::debug!(target: LOG_TARGET, "[workflow_run_engine] resume.spawned run={id}");
    Ok(resumed)
}

/// Build the root parent context + drive the phase DAG to completion.
///
/// Separated from [`start_workflow_run`] so it can run on the spawned task with
/// an owned [`Config`]. Errors are recorded on the run row (status `Failed`)
/// rather than propagated — there is no caller to receive them.
pub(crate) async fn run_engine_loop(
    ctx: &DispatchCtx,
    config: &Config,
    run_id: &str,
    definition: WorkflowDefinition,
    cancel: WorkflowCancelSignal,
    parent_override: Option<ParentExecutionContext>,
) {
    let model_override = get_workflow_run(config, run_id)
        .ok()
        .flatten()
        .and_then(|run| {
            run.input
                .get("modelOverride")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .filter(|model| !model.trim().is_empty());
    let safety_tier = match super::super::host::admit_workflow(&definition) {
        Ok(tier) => tier,
        Err(error) => {
            log::error!(target: LOG_TARGET, "[workflow_run_engine] loop.safety_admission_failed run={run_id} err={error}");
            if let Ok(Some(run)) = get_workflow_run(config, run_id) {
                let _ = compare_and_swap_workflow_run_lifecycle(
                    config,
                    WorkflowRunUpsert {
                        id: run.id.clone(),
                        definition_id: run.definition_id.clone(),
                        parent_thread_id: run.parent_thread_id.clone(),
                        input: run.input.clone(),
                        phase_states: run.phase_states.clone(),
                        child_run_ids: run.child_run_ids.clone(),
                        status: WorkflowRunStatus::Failed,
                        summary: Some(format!("workflow safety admission failed: {error}")),
                        started_at: Some(run.started_at),
                        completed_at: Some(chrono::Utc::now()),
                    },
                    run.revision,
                );
            }
            clear_cancel_signal(run_id, &cancel);
            return;
        }
    };
    let store = match workflow_store(config) {
        Ok(store) => store,
        Err(error) => {
            log::error!(target: LOG_TARGET, "[workflow_run_engine] loop.no_store run={run_id} err={error:#}");
            clear_cancel_signal(run_id, &cancel);
            return;
        }
    };
    let cancel_token = cancel.token.clone();
    let outcome = with_root_parent(
        ctx,
        parent_override,
        config,
        "workflow_engine",
        "workflow",
        "workflow",
        |parent| async move {
            let engine = WorkflowEngine::new(
                store,
                Arc::new(super::super::host::OpenHumanWorkflowExecutor::new(
                    run_id,
                    model_override,
                    safety_tier,
                    Some(parent),
                )),
            )
            .with_event_sink(Arc::new(tinyagents_graph::GraphTracingSink::new(format!(
                "workflow:{run_id}"
            ))));
            engine
                .drive(run_id, &definition, cancel_token)
                .await
                .map_err(anyhow::Error::msg)
        },
    )
    .await
    // Flatten: outer Err = root-parent build failure, inner = drive_phases result.
    .unwrap_or_else(Err);

    if let Err(err) = outcome {
        if !is_current_cancel_signal(run_id, &cancel) {
            log::debug!(
                target: LOG_TARGET,
                "[workflow_run_engine] loop.owner_lost run={run_id}; suppressing stale failure"
            );
            return;
        }
        log::error!(
            target: LOG_TARGET,
            "[workflow_run_engine] loop.failed run={run_id} err={err}"
        );
        // Best-effort terminal failure write, preserving partial phase state.
        if let Ok(Some(run)) = get_workflow_run(config, run_id) {
            if !matches!(
                run.status,
                WorkflowRunStatus::Completed
                    | WorkflowRunStatus::Failed
                    | WorkflowRunStatus::Cancelled
                    | WorkflowRunStatus::Interrupted
            ) {
                let _ = compare_and_swap_workflow_run_lifecycle(
                    config,
                    WorkflowRunUpsert {
                        id: run.id.clone(),
                        definition_id: run.definition_id.clone(),
                        parent_thread_id: run.parent_thread_id.clone(),
                        input: run.input.clone(),
                        phase_states: run.phase_states.clone(),
                        child_run_ids: run.child_run_ids.clone(),
                        status: WorkflowRunStatus::Failed,
                        summary: Some(format!("engine error: {err}")),
                        started_at: Some(run.started_at),
                        completed_at: Some(chrono::Utc::now()),
                    },
                    run.revision,
                );
            }
        }
    }

    clear_cancel_signal(run_id, &cancel);
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
