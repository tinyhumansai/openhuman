
/// `run_phase` step: mark the phase running, fan its agents out on the
/// intra-phase tinyagents graph (bounded by `default_concurrency`, capped by the
/// run-wide `max_children` budget), aggregate outcomes, and persist the new
/// phase state. Returns [`PhaseExecOutcome::Continue`] with the number of
/// children spawned, or [`PhaseExecOutcome::Terminated`] when the phase failed or
/// cancellation landed mid-phase (the terminal status is persisted first).
#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_phase(
    config: &Config,
    run_id: &str,
    definition: &WorkflowDefinition,
    session: &crate::openhuman::agent::orchestration::AgentOrchestrationSession,
    cancel: &Arc<AtomicBool>,
    model_override: Option<String>,
    phase: &WorkflowPhase,
    total_spawned: u32,
) -> Result<PhaseExecOutcome> {
    use crate::openhuman::agent::orchestration::{
        OrchestrationTaskStatus, SpawnAgentRequest, WaitAgentOptions,
    };

    // Reload so the phase state we mutate + persist is the latest projection.
    let run = get_workflow_run(&config.workspace_dir, run_id)?
        .ok_or_else(|| anyhow!("workflow run {run_id} vanished mid-phase"))?;
    let mut phase_states = run.phase_states.clone();
    let mut child_run_ids = run.child_run_ids.clone();
    // Children launched *this* phase (the reducer delta added to `total_spawned`).
    let mut spawned_this_phase: u32 = 0;

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

    // Run the phase's agents on a tinyagents graph fan-out (dispatch ->
    // parallel worker nodes bounded by `default_concurrency` -> collect
    // barrier), never exceeding the run-wide `max_children` cap. Each worker
    // spawns one child and waits for its terminal status; outcomes return in
    // phase order.
    let mut phase_outputs: Vec<Value> = Vec::new();
    let mut phase_failed: Option<String> = None;

    let concurrency = definition.default_concurrency.max(1) as usize;
    let budget_left = definition.max_children.saturating_sub(total_spawned);

    if budget_left == 0 {
        phase_failed = Some(format!(
            "max_children cap ({}) reached before phase '{}' completed",
            definition.max_children, phase.name
        ));
    } else {
        // Cap this phase to the run-wide `max_children` budget; if the phase
        // needs more workers than the budget allows we run as many as fit
        // and then fail with the cap message (matching the legacy loop).
        let capacity = budget_left as usize;
        let phase_agents = phase.agent_ids.to_vec();
        let capped = phase_agents.len() > capacity;
        let to_run: Vec<(usize, String)> = phase_agents
            .into_iter()
            .take(capacity)
            .enumerate()
            .collect();

        // Clones moved into the (`'static`) worker closure.
        let session_for_workers = session.clone();
        let cancel_for_workers = cancel.clone();
        let run_input = run.input.clone();
        let phase_owned = phase.clone();
        let upstream_owned = upstream_context.clone();
        let model_for_workers = model_override.clone();

        tracing::debug!(
            target: "orchestration",
            workers = to_run.len(),
            max_concurrency = concurrency,
            "[orchestration] running parallel fan-out on tinyagents map_reduce (workflow:{run_id}:{})",
            phase_owned.name
        );
        let expected_outcomes = to_run.len();
        let mut options = ParallelOptions::default()
            .with_max_concurrency(concurrency)
            .with_failure_policy(FailurePolicy::CollectAll);
        if let Some(token) = lookup_cancel_token(run_id) {
            options = options.with_cancellation(token);
        }
        let outcome = match map_reduce(to_run, options, move |_node, (agent_index, agent_id)| {
            let session = session_for_workers.clone();
            let cancel = cancel_for_workers.clone();
            let run_input = run_input.clone();
            let phase = phase_owned.clone();
            let upstream = upstream_owned.clone();
            let model = model_for_workers.clone();
            async move {
                // Don't launch new children once cancellation has landed.
                if cancel.load(Ordering::SeqCst) {
                    return Ok(PhaseWorkerOutcome {
                        orchestration_id: None,
                        output: None,
                        error: Some("cancelled before spawn".to_string()),
                    });
                }
                let prompt = phase_prompt(&run_input, &phase, agent_index, &upstream);
                let resp = match session
                    .spawn_agent(SpawnAgentRequest {
                        agent_id: agent_id.clone(),
                        prompt,
                        model,
                        ..Default::default()
                    })
                    .await
                {
                    Ok(resp) => resp,
                    Err(err) => {
                        return Ok(PhaseWorkerOutcome {
                            orchestration_id: None,
                            output: None,
                            error: Some(format!("spawn failed for agent '{agent_id}': {err}")),
                        });
                    }
                };
                let oid = resp.orchestration_id.clone();
                let wait = match session
                    .wait_agents(WaitAgentOptions {
                        orchestration_ids: vec![oid.clone()],
                        timeout_ms: None,
                    })
                    .await
                {
                    Ok(w) => w,
                    Err(err) => {
                        return Ok(PhaseWorkerOutcome {
                            orchestration_id: Some(oid),
                            output: None,
                            error: Some(format!("wait_agents failed: {err}")),
                        });
                    }
                };
                Ok(match wait.agents.into_iter().next() {
                    Some(s) => match s.status {
                        OrchestrationTaskStatus::Completed => PhaseWorkerOutcome {
                            orchestration_id: Some(oid),
                            output: Some(json!({
                                "orchestrationId": s.orchestration_id,
                                "agentId": s.agent_id,
                                "output": s.result_summary.clone().unwrap_or_default(),
                            })),
                            error: None,
                        },
                        OrchestrationTaskStatus::Failed
                        | OrchestrationTaskStatus::Cancelled
                        | OrchestrationTaskStatus::CancelRequested
                        | OrchestrationTaskStatus::TimedOut
                        | OrchestrationTaskStatus::Abandoned => PhaseWorkerOutcome {
                            orchestration_id: Some(oid),
                            output: None,
                            error: Some(format!(
                                "child '{}' (agent '{}') ended {}: {}",
                                s.orchestration_id,
                                s.agent_id,
                                serde_json::to_value(s.status)
                                    .ok()
                                    .and_then(|v| v.as_str().map(str::to_string))
                                    .unwrap_or_else(|| "non-completed".to_string()),
                                s.error.clone().unwrap_or_default()
                            )),
                        },
                        OrchestrationTaskStatus::Pending
                        | OrchestrationTaskStatus::Running
                        | OrchestrationTaskStatus::Awaiting => PhaseWorkerOutcome {
                            orchestration_id: Some(oid),
                            output: None,
                            error: Some(format!(
                                "child '{}' returned non-terminal status",
                                s.orchestration_id
                            )),
                        },
                    },
                    None => PhaseWorkerOutcome {
                        orchestration_id: Some(oid),
                        output: None,
                        error: Some("child returned no snapshot".to_string()),
                    },
                })
            }
        })
        .await
        {
            Ok(outcome) => outcome,
            Err(TinyAgentsError::Cancelled) => {
                log::debug!(
                    target: LOG_TARGET,
                    "[workflow_run_engine] phase.cancelled_by_sdk run={run_id} phase={}",
                    phase.name
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
                return Ok(PhaseExecOutcome::Terminated);
            }
            Err(err) => return Err(anyhow!("workflow fan-out failed: {err}")),
        };

        let mut outcomes = Vec::with_capacity(expected_outcomes);
        for item in outcome.outcomes {
            match item.result {
                Ok(value) => outcomes.push(value),
                Err(err) => {
                    return Err(anyhow!(
                        "workflow fan-out: worker {} failed: {err}",
                        item.index
                    ));
                }
            }
        }
        if outcomes.len() != expected_outcomes {
            return Err(anyhow!(
                "workflow fan-out: expected {expected_outcomes} result(s), got {}",
                outcomes.len()
            ));
        }

        // Aggregate worker outcomes in phase order: record every spawned
        // child id, collect completed outputs, and surface the first failure.
        for outcome in outcomes {
            if let Some(oid) = outcome.orchestration_id {
                spawned_this_phase += 1;
                child_run_ids.push(oid);
            }
            match outcome.output {
                Some(out) => phase_outputs.push(out),
                None => {
                    if phase_failed.is_none() {
                        phase_failed = outcome.error;
                    }
                }
            }
        }

        // Cancellation landed mid-phase: abort stragglers and interrupt.
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
            return Ok(PhaseExecOutcome::Terminated);
        }

        if capped && phase_failed.is_none() {
            phase_failed = Some(format!(
                "max_children cap ({}) reached before phase '{}' completed",
                definition.max_children, phase.name
            ));
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
        return Ok(PhaseExecOutcome::Terminated);
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

    Ok(PhaseExecOutcome::Continue {
        spawned: spawned_this_phase,
    })
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
        &config.workspace_dir,
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
