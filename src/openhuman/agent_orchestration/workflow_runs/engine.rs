//! Live execution engine for durable workflow runs (#3375 PR2).
//!
//! PR1 shipped the declarative [`WorkflowDefinition`] model, the durable
//! [`WorkflowRun`] ledger, and the read controllers. This module makes runs
//! actually *execute*: [`start_workflow_run`] resolves a definition, creates a
//! `Running` ledger row, and spawns a non-blocking engine task that walks the
//! phase DAG in dependency order, fanning out each phase's agents through the
//! programmatic [`AgentOrchestrationSession`] with bounded concurrency, then
//! persisting phase outputs after every phase. [`stop_workflow_run`] flips a
//! cancellation signal the loop checks between phases (→ `Interrupted`);
//! [`resume_workflow_run`] reloads a run and continues from the first
//! incomplete phase.
//!
//! ## Root parent context (the one real unknown)
//!
//! Child agents are spawned via [`AgentOrchestrationSession::spawn_agent`],
//! which reads the *parent execution context* from a task-local
//! ([`current_parent`]). The engine runs from a controller-spawned background
//! task — there is **no** agent turn on the stack, so the task-local is unset
//! and a naive spawn would fail with `NoParentContext`.
//!
//! The fix mirrors the production blueprint in
//! [`crate::openhuman::agent::triage::escalation`]: build a *root*
//! [`ParentExecutionContext`] from a real [`Agent`] (`Agent::from_config`) and
//! run the whole phase loop inside [`with_parent_context`]. Every
//! `spawn_agent` call nested in that scope then resolves `current_parent()` to
//! the root, inheriting a real provider, tool registry, memory, and model — the
//! same construction path `agent_chat` uses.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

use crate::openhuman::agent::harness::fork_context::with_parent_context;
use crate::openhuman::agent_orchestration::parent_context::build_root_parent;
use crate::openhuman::config::Config;
use crate::openhuman::session_db::run_ledger::{
    get_workflow_run, upsert_workflow_run, WorkflowRun, WorkflowRunStatus, WorkflowRunUpsert,
};

use super::ops::definition_by_id;
use super::types::{WorkflowDefinition, WorkflowPhase};

const LOG_TARGET: &str = "workflow_run_engine";

/// Per-phase status stored inside the run's `phase_states` JSON column.
const PHASE_PENDING: &str = "pending";
const PHASE_RUNNING: &str = "running";
const PHASE_COMPLETED: &str = "completed";
const PHASE_FAILED: &str = "failed";

// ───────────────────────────────────────────────────────────────────────────
// Cancellation registry
// ───────────────────────────────────────────────────────────────────────────

/// Process-wide map of `run_id -> cancellation flag`. `stop_workflow_run`
/// flips the flag; the engine loop checks it between phases and aborts in-flight
/// child tasks via the orchestration session before marking the run
/// `Interrupted`.
fn cancel_registry() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register (or reuse) a cancellation flag for `run_id`.
fn register_cancel_flag(run_id: &str) -> Arc<AtomicBool> {
    let mut map = cancel_registry().lock().expect("cancel registry poisoned");
    map.entry(run_id.to_string())
        .or_insert_with(|| Arc::new(AtomicBool::new(false)))
        .clone()
}

/// Look up an existing cancellation flag for `run_id`, if one is registered.
fn lookup_cancel_flag(run_id: &str) -> Option<Arc<AtomicBool>> {
    cancel_registry()
        .lock()
        .expect("cancel registry poisoned")
        .get(run_id)
        .cloned()
}

/// Drop a run's cancellation flag once the engine loop is done with it.
fn clear_cancel_flag(run_id: &str) {
    cancel_registry()
        .lock()
        .expect("cancel registry poisoned")
        .remove(run_id);
}

// ───────────────────────────────────────────────────────────────────────────
// Public entry points
// ───────────────────────────────────────────────────────────────────────────

/// Start a new workflow run and return immediately.
///
/// Resolves `definition_id` to a builtin [`WorkflowDefinition`], creates a
/// `Running` ledger row with `phase_states` initialised to one `pending` entry
/// per phase, persists it, then `tokio::spawn`s the engine loop. The returned
/// [`WorkflowRun`] is the freshly-created row (status `Running`); callers poll
/// `workflow_run_get` to observe progress.
pub async fn start_workflow_run(
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

    let run_id = format!("wfrun-{}", uuid::Uuid::new_v4());
    let phase_states = init_phase_states(&definition);

    let run = upsert_workflow_run(
        config,
        WorkflowRunUpsert {
            id: run_id.clone(),
            definition_id: definition.id.clone(),
            parent_thread_id,
            input: input.clone(),
            phase_states,
            child_run_ids: Vec::new(),
            status: WorkflowRunStatus::Running,
            summary: None,
            started_at: None,
            completed_at: None,
        },
    )
    .context("persist initial workflow run")?;

    register_cancel_flag(&run_id);

    // Spawn the engine loop. Clone what the task needs (the engine reloads
    // config inside the task so it can build a real Agent without holding a
    // borrow across the spawn boundary).
    let task_run_id = run_id.clone();
    tokio::spawn(async move {
        match Config::load_or_init().await {
            Ok(task_config) => {
                run_engine_loop(&task_config, &task_run_id, definition).await;
            }
            Err(err) => {
                log::error!(
                    target: LOG_TARGET,
                    "[workflow_run_engine] start.config_load_failed run={task_run_id} err={err}"
                );
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

    if let Some(flag) = lookup_cancel_flag(id) {
        flag.store(true, Ordering::SeqCst);
    } else {
        // No live loop (e.g. process restart) — register a flag anyway so a
        // future resume observes the stop intent.
        register_cancel_flag(id).store(true, Ordering::SeqCst);
    }

    let updated = upsert_workflow_run(
        config,
        WorkflowRunUpsert {
            id: run.id.clone(),
            definition_id: run.definition_id.clone(),
            parent_thread_id: run.parent_thread_id.clone(),
            input: run.input.clone(),
            phase_states: run.phase_states.clone(),
            child_run_ids: run.child_run_ids.clone(),
            status: WorkflowRunStatus::Interrupted,
            summary: run.summary.clone(),
            started_at: Some(run.started_at),
            completed_at: None,
        },
    )
    .context("persist workflow run interrupt")?;

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
pub async fn resume_workflow_run(config: &Config, id: &str) -> Result<WorkflowRun> {
    log::debug!(target: LOG_TARGET, "[workflow_run_engine] resume.entry run={id}");
    let run = get_workflow_run(config, id)?.ok_or_else(|| anyhow!("unknown workflow run: {id}"))?;

    if matches!(run.status, WorkflowRunStatus::Completed) {
        return Err(anyhow!("workflow run {id} is already completed"));
    }

    let definition = definition_by_id(&run.definition_id)
        .ok_or_else(|| anyhow!("definition {} no longer exists", run.definition_id))?;

    // Clear any prior cancellation intent and re-register a fresh flag.
    clear_cancel_flag(id);
    register_cancel_flag(id);

    let resumed = upsert_workflow_run(
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
    )
    .context("persist workflow run resume")?;

    let task_run_id = id.to_string();
    tokio::spawn(async move {
        match Config::load_or_init().await {
            Ok(task_config) => {
                run_engine_loop(&task_config, &task_run_id, definition).await;
            }
            Err(err) => {
                log::error!(
                    target: LOG_TARGET,
                    "[workflow_run_engine] resume.config_load_failed run={task_run_id} err={err}"
                );
            }
        }
    });

    log::debug!(target: LOG_TARGET, "[workflow_run_engine] resume.spawned run={id}");
    Ok(resumed)
}

// ───────────────────────────────────────────────────────────────────────────
// Engine loop
// ───────────────────────────────────────────────────────────────────────────

/// Build the root parent context + drive the phase DAG to completion.
///
/// Separated from [`start_workflow_run`] so it can run on the spawned task with
/// an owned [`Config`]. Errors are recorded on the run row (status `Failed`)
/// rather than propagated — there is no caller to receive them.
async fn run_engine_loop(config: &Config, run_id: &str, definition: WorkflowDefinition) {
    let cancel = lookup_cancel_flag(run_id).unwrap_or_else(|| register_cancel_flag(run_id));

    let outcome = match build_root_parent(config, "workflow_engine", "workflow", "workflow").await {
        Ok(parent) => {
            with_parent_context(parent, async {
                drive_phases(config, run_id, &definition, &cancel).await
            })
            .await
        }
        Err(err) => Err(err),
    };

    if let Err(err) = outcome {
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
                let _ = persist(
                    config,
                    &run,
                    run.phase_states.clone(),
                    run.child_run_ids.clone(),
                    WorkflowRunStatus::Failed,
                    Some(format!("engine error: {err}")),
                    true,
                );
            }
        }
    }

    clear_cancel_flag(run_id);
}

/// Topologically walk the phase DAG, executing each phase whose dependencies
/// are all `completed`. Persists after every phase. Returns `Ok(())` once every
/// phase is completed, the run is interrupted, or a phase fails (the terminal
/// status is written to the row before returning `Ok`). Returns `Err` only for
/// engine-internal failures (e.g. ledger write errors).
async fn drive_phases(
    config: &Config,
    run_id: &str,
    definition: &WorkflowDefinition,
    cancel: &Arc<AtomicBool>,
) -> Result<()> {
    use crate::openhuman::agent_orchestration::{
        AgentOrchestrationSession, AgentStatus, SpawnAgentRequest, WaitAgentOptions,
    };

    let session = AgentOrchestrationSession::new(format!("workflow-engine-{run_id}"));
    let mut total_spawned: u32 = 0;

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

    loop {
        // Reload the run each iteration so we read the latest phase_states (and
        // so a resume picks up persisted progress).
        let run = get_workflow_run(config, run_id)?
            .ok_or_else(|| anyhow!("workflow run {run_id} vanished mid-loop"))?;
        let mut phase_states = run.phase_states.clone();
        let mut child_run_ids = run.child_run_ids.clone();

        // Cancellation check between phases.
        if cancel.load(Ordering::SeqCst) {
            log::debug!(
                target: LOG_TARGET,
                "[workflow_run_engine] loop.cancelled run={run_id}"
            );
            session.abort_all().await;
            persist(
                config,
                &run,
                phase_states,
                child_run_ids,
                WorkflowRunStatus::Interrupted,
                None,
                false,
            )?;
            return Ok(());
        }

        // Find the next runnable phase: pending, with all deps completed.
        let Some(phase) = next_runnable_phase(definition, &phase_states) else {
            // No runnable phase left. Either everything is done, or we're
            // blocked (which a validated DAG shouldn't be).
            if all_phases_completed(definition, &phase_states) {
                let summary = synthesize_summary(definition, &phase_states);
                log::debug!(
                    target: LOG_TARGET,
                    "[workflow_run_engine] loop.completed run={run_id} summary_chars={}",
                    summary.as_deref().map(str::len).unwrap_or(0)
                );
                persist(
                    config,
                    &run,
                    phase_states,
                    child_run_ids,
                    WorkflowRunStatus::Completed,
                    summary,
                    true,
                )?;
            } else {
                log::warn!(
                    target: LOG_TARGET,
                    "[workflow_run_engine] loop.stuck run={run_id} no_runnable_phase"
                );
                persist(
                    config,
                    &run,
                    phase_states,
                    child_run_ids,
                    WorkflowRunStatus::Failed,
                    Some("no runnable phase (dependency deadlock)".to_string()),
                    true,
                )?;
            }
            return Ok(());
        };

        log::debug!(
            target: LOG_TARGET,
            "[workflow_run_engine] phase.start run={run_id} phase={} agents={} spawned_so_far={}",
            phase.name,
            phase.agent_ids.len(),
            total_spawned
        );
        set_phase_status(&mut phase_states, &phase.name, PHASE_RUNNING, None);
        persist(
            config,
            &run,
            phase_states.clone(),
            child_run_ids.clone(),
            WorkflowRunStatus::Running,
            None,
            false,
        )?;

        // Thread prior phases' outputs into this phase's prompt context.
        let upstream_context = upstream_outputs(definition, phase, &phase_states);

        // Spawn the phase's agents in concurrency-bounded batches, never
        // exceeding the run-wide `max_children` cap.
        let mut phase_outputs: Vec<Value> = Vec::new();
        let mut phase_failed: Option<String> = None;
        let mut index_in_phase = 0usize;

        let concurrency = definition.default_concurrency.max(1);
        let mut pending = phase.agent_ids.to_vec();

        'phase: while !pending.is_empty() {
            // Cancellation can also land mid-phase between batches.
            if cancel.load(Ordering::SeqCst) {
                session.abort_all().await;
                persist(
                    config,
                    &run,
                    phase_states,
                    child_run_ids,
                    WorkflowRunStatus::Interrupted,
                    None,
                    false,
                )?;
                return Ok(());
            }

            let budget_left = definition.max_children.saturating_sub(total_spawned);
            if budget_left == 0 {
                phase_failed = Some(format!(
                    "max_children cap ({}) reached before phase '{}' completed",
                    definition.max_children, phase.name
                ));
                break 'phase;
            }
            let batch_size = (concurrency as usize)
                .min(budget_left as usize)
                .min(pending.len())
                .max(1);
            let batch: Vec<String> = pending.drain(..batch_size).collect();

            let mut batch_ids: Vec<(String, String)> = Vec::new(); // (orchestration_id, agent_id)
            for agent_id in &batch {
                let prompt = phase_prompt(&run.input, phase, index_in_phase, &upstream_context);
                index_in_phase += 1;
                match session
                    .spawn_agent(SpawnAgentRequest {
                        agent_id: agent_id.clone(),
                        prompt,
                        model: model_override.clone(),
                        ..Default::default()
                    })
                    .await
                {
                    Ok(resp) => {
                        total_spawned += 1;
                        child_run_ids.push(resp.orchestration_id.clone());
                        batch_ids.push((resp.orchestration_id, agent_id.clone()));
                    }
                    Err(err) => {
                        phase_failed = Some(format!("spawn failed for agent '{agent_id}': {err}"));
                        break 'phase;
                    }
                }
            }

            // Wait for this batch to reach terminal status.
            let wait = session
                .wait_agents(WaitAgentOptions {
                    orchestration_ids: batch_ids.iter().map(|(id, _)| id.clone()).collect(),
                    timeout_ms: None,
                })
                .await
                .map_err(|e| anyhow!("wait_agents failed: {e}"))?;

            for snapshot in &wait.agents {
                match snapshot.status {
                    AgentStatus::Completed => {
                        phase_outputs.push(json!({
                            "orchestrationId": snapshot.orchestration_id,
                            "agentId": snapshot.agent_id,
                            "output": snapshot.result_summary.clone().unwrap_or_default(),
                        }));
                    }
                    AgentStatus::Failed | AgentStatus::Cancelled | AgentStatus::Closed => {
                        phase_failed = Some(format!(
                            "child '{}' (agent '{}') ended {}: {}",
                            snapshot.orchestration_id,
                            snapshot.agent_id,
                            serde_json::to_value(snapshot.status)
                                .ok()
                                .and_then(|v| v.as_str().map(str::to_string))
                                .unwrap_or_else(|| "non-completed".to_string()),
                            snapshot.error.clone().unwrap_or_default()
                        ));
                        break 'phase;
                    }
                    AgentStatus::Pending | AgentStatus::Running | AgentStatus::Waiting => {
                        // wait_agents with no timeout only returns on terminal,
                        // so this is defensive.
                        phase_failed = Some(format!(
                            "child '{}' returned non-terminal status",
                            snapshot.orchestration_id
                        ));
                        break 'phase;
                    }
                }
            }
        }

        if let Some(reason) = phase_failed {
            log::warn!(
                target: LOG_TARGET,
                "[workflow_run_engine] phase.failed run={run_id} phase={} reason={reason}",
                phase.name
            );
            set_phase_status(
                &mut phase_states,
                &phase.name,
                PHASE_FAILED,
                Some(json!([])),
            );
            set_phase_reason(&mut phase_states, &phase.name, &reason);
            persist(
                config,
                &run,
                phase_states,
                child_run_ids,
                WorkflowRunStatus::Failed,
                Some(reason),
                true,
            )?;
            return Ok(());
        }

        log::debug!(
            target: LOG_TARGET,
            "[workflow_run_engine] phase.done run={run_id} phase={} outputs={}",
            phase.name,
            phase_outputs.len()
        );
        set_phase_status(
            &mut phase_states,
            &phase.name,
            PHASE_COMPLETED,
            Some(Value::Array(phase_outputs)),
        );
        persist(
            config,
            &run,
            phase_states,
            child_run_ids,
            WorkflowRunStatus::Running,
            None,
            false,
        )?;
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Phase-state helpers
// ───────────────────────────────────────────────────────────────────────────

/// Initialise `phase_states` to one `pending` entry per phase, preserving
/// declaration order via an object keyed by phase name.
fn init_phase_states(definition: &WorkflowDefinition) -> Value {
    let mut map = serde_json::Map::new();
    for phase in &definition.phases {
        map.insert(
            phase.name.clone(),
            json!({ "status": PHASE_PENDING, "outputs": [] }),
        );
    }
    Value::Object(map)
}

fn phase_status<'a>(phase_states: &'a Value, name: &str) -> Option<&'a str> {
    phase_states
        .get(name)
        .and_then(|p| p.get("status"))
        .and_then(Value::as_str)
}

fn set_phase_status(phase_states: &mut Value, name: &str, status: &str, outputs: Option<Value>) {
    if let Some(obj) = phase_states.as_object_mut() {
        let entry = obj
            .entry(name.to_string())
            .or_insert_with(|| json!({ "status": PHASE_PENDING, "outputs": [] }));
        if let Some(entry_obj) = entry.as_object_mut() {
            entry_obj.insert("status".to_string(), json!(status));
            if let Some(out) = outputs {
                entry_obj.insert("outputs".to_string(), out);
            }
        }
    }
}

fn set_phase_reason(phase_states: &mut Value, name: &str, reason: &str) {
    if let Some(obj) = phase_states.as_object_mut() {
        if let Some(entry) = obj.get_mut(name).and_then(Value::as_object_mut) {
            entry.insert("reason".to_string(), json!(reason));
        }
    }
}

/// The first phase that is `pending` (or missing) and whose every dependency is
/// `completed`. Definition order breaks ties so the walk is deterministic.
fn next_runnable_phase<'a>(
    definition: &'a WorkflowDefinition,
    phase_states: &Value,
) -> Option<&'a WorkflowPhase> {
    definition.phases.iter().find(|phase| {
        let status = phase_status(phase_states, &phase.name).unwrap_or(PHASE_PENDING);
        if status == PHASE_COMPLETED || status == PHASE_RUNNING {
            return false;
        }
        phase
            .depends_on
            .iter()
            .all(|dep| phase_status(phase_states, dep) == Some(PHASE_COMPLETED))
    })
}

fn all_phases_completed(definition: &WorkflowDefinition, phase_states: &Value) -> bool {
    definition
        .phases
        .iter()
        .all(|phase| phase_status(phase_states, &phase.name) == Some(PHASE_COMPLETED))
}

/// Collect the outputs of every completed phase this phase depends on, so they
/// can be threaded into the downstream prompt.
fn upstream_outputs(
    _definition: &WorkflowDefinition,
    phase: &WorkflowPhase,
    phase_states: &Value,
) -> Vec<Value> {
    let mut out = Vec::new();
    for dep in &phase.depends_on {
        if let Some(outputs) = phase_states
            .get(dep)
            .and_then(|p| p.get("outputs"))
            .and_then(Value::as_array)
        {
            for item in outputs {
                if let Some(text) = item.get("output").and_then(Value::as_str) {
                    if !text.trim().is_empty() {
                        out.push(json!({ "phase": dep, "output": text }));
                    }
                }
            }
        }
    }
    out
}

/// Build the prompt for one child in a phase: the run input + the phase's
/// description + upstream outputs threaded in as context.
fn phase_prompt(
    input: &Value,
    phase: &WorkflowPhase,
    index_in_phase: usize,
    upstream: &[Value],
) -> String {
    let question = input
        .get("question")
        .or_else(|| input.get("input"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| input.to_string());

    let mut prompt = format!(
        "Workflow phase: {}\n{}\n\nInput:\n{}\n",
        phase.name, phase.description, question
    );
    if phase.agent_ids.len() > 1 {
        prompt.push_str(&format!(
            "\n(You are worker #{} in this phase.)\n",
            index_in_phase + 1
        ));
    }
    if !upstream.is_empty() {
        prompt.push_str("\nContext from prior phases:\n");
        for item in upstream {
            if let (Some(p), Some(o)) = (
                item.get("phase").and_then(Value::as_str),
                item.get("output").and_then(Value::as_str),
            ) {
                prompt.push_str(&format!("- [{p}] {o}\n"));
            }
        }
    }
    prompt
}

/// The synthesize phase's combined output becomes the run summary. Falls back
/// to the last completed phase's output if no phase is literally named
/// `synthesize`.
fn synthesize_summary(definition: &WorkflowDefinition, phase_states: &Value) -> Option<String> {
    let pick = |name: &str| -> Option<String> {
        let outputs = phase_states
            .get(name)
            .and_then(|p| p.get("outputs"))
            .and_then(Value::as_array)?;
        let joined = outputs
            .iter()
            .filter_map(|o| o.get("output").and_then(Value::as_str))
            .filter(|s| !s.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        (!joined.trim().is_empty()).then_some(joined)
    };

    if let Some(summary) = pick("synthesize") {
        return Some(summary);
    }
    // Fall back to the last phase in declaration order with non-empty output.
    definition
        .phases
        .iter()
        .rev()
        .find_map(|phase| pick(&phase.name))
}

/// Persist a run-state update. `terminal` controls whether `completed_at` is
/// stamped.
#[allow(clippy::too_many_arguments)]
fn persist(
    config: &Config,
    run: &WorkflowRun,
    phase_states: Value,
    child_run_ids: Vec<String>,
    status: WorkflowRunStatus,
    summary: Option<String>,
    terminal: bool,
) -> Result<WorkflowRun> {
    upsert_workflow_run(
        config,
        WorkflowRunUpsert {
            id: run.id.clone(),
            definition_id: run.definition_id.clone(),
            parent_thread_id: run.parent_thread_id.clone(),
            input: run.input.clone(),
            phase_states,
            child_run_ids,
            status,
            summary,
            started_at: Some(run.started_at),
            completed_at: terminal.then(chrono::Utc::now),
        },
    )
    .context("persist workflow run state")
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod engine_tests;
