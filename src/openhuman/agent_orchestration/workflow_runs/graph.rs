//! The **workflow scheduler graph** (issue #4249, Phase 4).
//!
//! This is the `workflow_runs` folder's `graph.rs` per the per-folder graph
//! convention: the durable workflow run's phase-DAG scheduler expressed as a
//! `tinyagents` conditional-routing graph. A `dispatch` node selects the next
//! runnable phase and a `run_phase` node executes it, looping
//! `dispatch ⇄ run_phase` until no phase remains, then routing to `done`.
//!
//! The phase-execution logic (`select_next_phase`, `execute_phase`) and the run
//! lifecycle (`start_workflow_run` / `stop_workflow_run` / `resume_workflow_run`)
//! stay in [`super::engine`]; this module owns only the graph state machine that
//! drives them.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde_json::Value;

use crate::openhuman::config::Config;
use crate::openhuman::session_db::run_ledger::get_workflow_run;

use super::engine::{execute_phase, select_next_phase, PhaseExecOutcome, PhaseSelection};
use super::types::{WorkflowDefinition, WorkflowPhase};

/// Lift an engine-internal `anyhow` error into the scheduler graph's error type
/// so a ledger-write/spawn failure fails the run (and propagates back out via
/// [`drive_phases`]). A *phase* that merely failed is not an error here — it is a
/// normal `Terminated` outcome that persists `Failed` and routes to `done`.
fn graph_err(e: anyhow::Error) -> tinyagents::TinyAgentsError {
    tinyagents::TinyAgentsError::Graph(e.to_string())
}

/// Typed state threaded through the scheduler graph: the phase `dispatch`
/// selected for `run_phase`, and the running tally of spawned children (the
/// `max_children` budget counter, carried across the dispatch⇄run_phase cycle).
#[derive(Clone, Default)]
struct SchedulerState {
    phase: Option<WorkflowPhase>,
    total_spawned: u32,
}

/// Reducer updates emitted by the scheduler graph nodes.
enum SchedulerUpdate {
    /// `dispatch` chose the next phase to run.
    SelectPhase(WorkflowPhase),
    /// `run_phase` spawned this many children; fold into `total_spawned`.
    AddSpawned(u32),
    /// Terminal node fired; no state change.
    Noop,
}

/// Topologically walk the phase DAG on a `tinyagents` conditional-routing graph
/// (issue #4249, Phase 4): a `dispatch` node selects the next runnable phase and
/// a `run_phase` node executes it, looping `dispatch ⇄ run_phase` until no phase
/// remains, then routing to `done`:
///
/// ```text
///   dispatch ──phase──► run_phase ──► dispatch ──none/terminal──► done
/// ```
///
/// This replaces the hand-rolled `loop {}` scheduler with a graph state machine
/// while keeping the durable `workflow_runs` row as the source of truth (every
/// node reloads + persists it, so resume picks up persisted phase progress and
/// the read controllers see the same projection). Returns `Ok(())` once a
/// terminal status (Completed / Failed / Interrupted) is written; returns `Err`
/// only for engine-internal failures (ledger writes, graph mechanics).
pub(super) async fn drive_phases(
    config: &Config,
    run_id: &str,
    definition: &WorkflowDefinition,
    cancel: &Arc<AtomicBool>,
) -> Result<()> {
    use crate::openhuman::agent_orchestration::AgentOrchestrationSession;
    use crate::openhuman::tinyagents::observability::GraphTracingSink;
    use tinyagents::graph::recursion::RecursionPolicy;
    use tinyagents::graph::{ClosureStateReducer, Command, GraphBuilder, NodeContext, NodeResult};

    let session = AgentOrchestrationSession::new(format!("workflow-engine-{run_id}"));

    // Optional per-run model override. When present in the run input
    // (`{"modelOverride": "..."}`), every child is forced onto the parent
    // (root) provider with this model instead of its declarative workload
    // provider. Production runs omit it (agents use their configured provider);
    // deterministic mock-backend tests set it so children resolve to the
    // injected mock provider.
    let model_override = get_workflow_run(config, run_id)?
        .and_then(|r| {
            r.input
                .get("modelOverride")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .filter(|m| !m.trim().is_empty());

    // `'static` captures for the graph node handlers (they are `Fn`, re-entered
    // once per phase, so each invocation re-clones from these).
    let config = Arc::new(config.clone());
    let definition = Arc::new(definition.clone());
    let run_id_owned = run_id.to_string();
    let cancel = cancel.clone();

    let mut builder = GraphBuilder::<SchedulerState, SchedulerUpdate>::new().set_reducer(
        ClosureStateReducer::new(|mut s: SchedulerState, u: SchedulerUpdate| {
            match u {
                SchedulerUpdate::SelectPhase(p) => s.phase = Some(p),
                SchedulerUpdate::AddSpawned(n) => s.total_spawned += n,
                SchedulerUpdate::Noop => {}
            }
            Ok(s)
        }),
    );

    // `dispatch`: pick the next runnable phase, or terminate (already persisted).
    {
        let config = config.clone();
        let definition = definition.clone();
        let run_id = run_id_owned.clone();
        let cancel = cancel.clone();
        let session = session.clone();
        builder = builder.add_node("dispatch", move |_s: SchedulerState, _c: NodeContext| {
            let config = config.clone();
            let definition = definition.clone();
            let run_id = run_id.clone();
            let cancel = cancel.clone();
            let session = session.clone();
            async move {
                match select_next_phase(&config, &run_id, &definition, &cancel, &session)
                    .await
                    .map_err(graph_err)?
                {
                    PhaseSelection::Run(phase) => Ok(NodeResult::Command(
                        Command::default()
                            .with_update(SchedulerUpdate::SelectPhase(phase))
                            .with_goto(["run_phase"]),
                    )),
                    PhaseSelection::Terminated => {
                        Ok(NodeResult::Command(Command::default().with_goto(["done"])))
                    }
                }
            }
        });
    }

    // `run_phase`: execute the selected phase, then loop back to `dispatch` or
    // terminate (on phase failure / mid-phase cancellation).
    {
        let config = config.clone();
        let definition = definition.clone();
        let run_id = run_id_owned.clone();
        let cancel = cancel.clone();
        let session = session.clone();
        let model_override = model_override.clone();
        builder = builder.add_node("run_phase", move |s: SchedulerState, _c: NodeContext| {
            let config = config.clone();
            let definition = definition.clone();
            let run_id = run_id.clone();
            let cancel = cancel.clone();
            let session = session.clone();
            let model_override = model_override.clone();
            async move {
                let phase = s.phase.clone().ok_or_else(|| {
                    tinyagents::TinyAgentsError::Graph(
                        "workflow run_phase reached with no selected phase".to_string(),
                    )
                })?;
                match execute_phase(
                    &config,
                    &run_id,
                    &definition,
                    &session,
                    &cancel,
                    model_override,
                    &phase,
                    s.total_spawned,
                )
                .await
                .map_err(graph_err)?
                {
                    PhaseExecOutcome::Continue { spawned } => Ok(NodeResult::Command(
                        Command::default()
                            .with_update(SchedulerUpdate::AddSpawned(spawned))
                            .with_goto(["dispatch"]),
                    )),
                    PhaseExecOutcome::Terminated => {
                        Ok(NodeResult::Command(Command::default().with_goto(["done"])))
                    }
                }
            }
        });
    }

    let phase_count = definition.phases.len();
    let graph = builder
        .add_node("done", |_s: SchedulerState, _c: NodeContext| async move {
            Ok(NodeResult::Update(SchedulerUpdate::Noop))
        })
        .set_entry("dispatch")
        .mark_command_routing("dispatch")
        .mark_command_routing("run_phase")
        .set_finish("done")
        .compile()
        .map_err(|e| anyhow!("workflow scheduler graph compile failed: {e}"))?
        .with_event_sink(Arc::new(GraphTracingSink::new(format!(
            "workflow:{run_id_owned}"
        ))))
        // Bound the dispatch⇄run_phase cycle as a backstop to the DAG's own
        // termination: `dispatch` is visited once per phase plus a final
        // no-phase visit, `run_phase` once per phase. A validated DAG always
        // drains, so this only guards a malformed definition.
        .with_recursion_policy(RecursionPolicy {
            max_visits_per_node: Some(phase_count + 2),
            max_total_steps: (phase_count + 1) * 3 + 16,
            ..RecursionPolicy::default()
        });

    graph
        .run(SchedulerState::default())
        .await
        .map_err(|e| anyhow!("workflow scheduler graph run failed: {e}"))?;
    Ok(())
}
