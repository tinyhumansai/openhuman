
async fn run_one_parallel_task(
    worker: SpawnParallelWorker,
    repo_root: Option<PathBuf>,
) -> ParallelAgentResult {
    let SpawnParallelWorker {
        definition,
        prompt,
        task,
        task_id,
        lineage,
        worktree_path,
        workspace_descriptor,
        dispatch_mode: _,
    } = worker;
    let started = std::time::Instant::now();
    tracing::debug!(
        task_id = %task_id,
        agent_id = %definition.id,
        toolkit = task.toolkit.as_deref().unwrap_or(""),
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
        toolkit_override: task.toolkit.clone(),
        context: task.context.clone(),
        model_override: None,
        task_id: Some(task_id.clone()),
        worker_thread_id: None,
        initial_history: None,
        checkpoint_dir: None,
        worktree_action_dir,
        workspace_descriptor,
        run_queue: None,
    };
    let run_result = run_subagent(&definition, &prompt, options).await;

    // After the worker finishes, snapshot the worktree's changed files +
    // dirty status so the parent can detect cross-worker overlaps and the UI
    // can surface diff/cleanup actions. Best-effort: a status error degrades
    // to "no changes recorded" rather than failing the task.
    let worktree_str = worktree_path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string());
    let (changed_files, dirty_status) = match (&worktree_path, &repo_root) {
        (Some(wt), Some(root)) => {
            use crate::openhuman::agent::orchestration::worktree;
            match worktree::status(root, wt) {
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
            tracing::debug!(
                task_id = %outcome.task_id,
                agent_id = %outcome.agent_id,
                elapsed_ms = outcome.elapsed.as_millis() as u64,
                iterations = outcome.iterations,
                output_chars = outcome.output.chars().count(),
                "[spawn_parallel_agents] task_success"
            );
            ParallelAgentResult {
                task_id: outcome.task_id,
                agent_id: outcome.agent_id,
                lineage,
                success: true,
                output: Some(outcome.output),
                error: None,
                ownership: task.ownership,
                elapsed_ms: outcome.elapsed.as_millis() as u64,
                iterations: outcome.iterations as u32,
                stale_parent_reads: Vec::new(),
                worktree_path: worktree_str,
                changed_files,
                dirty_status,
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
                output: None,
                error: Some(err.to_string()),
                ownership: task.ownership,
                elapsed_ms: started.elapsed().as_millis() as u64,
                iterations: 0,
                stale_parent_reads: Vec::new(),
                worktree_path: worktree_str,
                changed_files,
                dirty_status,
            }
        }
    }
}

const SPAWN_PARALLEL_PHASES: &[&str] = &["validate", "dispatch", "worker", "collect", "finalize"];

#[derive(Clone, Default)]
struct SpawnParallelState {
    visited: Vec<&'static str>,
    cancelled_phase: Option<&'static str>,
    tasks: Vec<ParallelAgentTask>,
    max_parallel: usize,
    rejection: Option<String>,
    prepared: Vec<SpawnParallelWorker>,
    immediate_results: Vec<ParallelAgentResult>,
    fanned_results: Vec<ParallelAgentResult>,
    results: Vec<ParallelAgentResult>,
    action_root: Option<PathBuf>,
    collected: Option<SpawnParallelCollected>,
}

impl SpawnParallelState {
    fn for_execution(
        tasks: Vec<ParallelAgentTask>,
        max_parallel: usize,
        action_root: Option<PathBuf>,
    ) -> Self {
        Self {
            tasks,
            max_parallel,
            action_root,
            ..Self::default()
        }
    }
}

enum SpawnParallelUpdate {
    PhaseEntered(&'static str),
    Cancelled(&'static str),
    Rejected(String),
    Staged {
        prepared: Vec<SpawnParallelWorker>,
        immediate_results: Vec<ParallelAgentResult>,
    },
    Fanned(Vec<ParallelAgentResult>),
    Results(Vec<ParallelAgentResult>),
    Collected(SpawnParallelCollected),
}

type SpawnParallelNodeFuture =
    Pin<Box<dyn Future<Output = tinyagents_harness::Result<NodeResult<SpawnParallelUpdate>>> + Send>>;

fn phase_node(
    phase: &'static str,
) -> impl Fn(SpawnParallelState, NodeContext) -> SpawnParallelNodeFuture + Clone + Send + Sync + 'static
{
    move |_state: SpawnParallelState, _ctx: NodeContext| {
        Box::pin(async move { Ok(NodeResult::Update(SpawnParallelUpdate::PhaseEntered(phase))) })
    }
}

/// Build the fixed `spawn_parallel_agents` graph scaffold.
///
/// The node order is intentionally static for topology export:
///
/// `validate -> dispatch -> worker -> collect -> finalize`
fn build_spawn_parallel_graph(
) -> Result<CompiledGraph<SpawnParallelState, SpawnParallelUpdate>, String> {
    let phases = SPAWN_PARALLEL_PHASES;
    GraphBuilder::<SpawnParallelState, SpawnParallelUpdate>::new()
        .set_reducer(ClosureStateReducer::new(
            |mut state: SpawnParallelState, update: SpawnParallelUpdate| {
                match update {
                    SpawnParallelUpdate::PhaseEntered(phase) => state.visited.push(phase),
                    SpawnParallelUpdate::Cancelled(phase) => {
                        state.visited.push(phase);
                        state.cancelled_phase.get_or_insert(phase);
                    }
                    SpawnParallelUpdate::Rejected(message) => state.rejection = Some(message),
                    SpawnParallelUpdate::Staged {
                        prepared,
                        immediate_results,
                    } => {
                        state.prepared = prepared;
                        state.immediate_results = immediate_results;
                    }
                    SpawnParallelUpdate::Fanned(results) => state.fanned_results = results,
                    SpawnParallelUpdate::Results(results) => state.results = results,
                    SpawnParallelUpdate::Collected(collected) => state.collected = Some(collected),
                }
                Ok(state)
            },
        ))
        .add_node(phases[0], phase_node(phases[0]))
        .add_node(phases[1], phase_node(phases[1]))
        .add_node(phases[2], phase_node(phases[2]))
        .add_node(phases[3], phase_node(phases[3]))
        .add_node(phases[4], phase_node(phases[4]))
        .add_edge(phases[0], phases[1])
        .add_edge(phases[1], phases[2])
        .add_edge(phases[2], phases[3])
        .add_edge(phases[3], phases[4])
        .set_entry(phases[0])
        .set_finish(phases[4])
        .compile()
        .map_err(|e| format!("spawn_parallel_agents graph compile failed: {e}"))
}

/// Run the fixed fanout graph over the live worker/collect/finalize phases.
///
/// Validation and worktree preflight still happen before this helper; the graph
/// owns the map-reduce worker fanout, compatibility progress projection, and
/// final result collection.
async fn run_spawn_parallel_execution_graph(
    parent_session: &str,
    progress_sink: Option<Sender<AgentProgress>>,
    tasks: Vec<ParallelAgentTask>,
    max_parallel: usize,
    definitions: HashMap<String, AgentDefinition>,
    parent: ParentExecutionContext,
    action_root: Option<PathBuf>,
    cancel: CancellationToken,
    parent_workspace_descriptor: Option<WorkspaceDescriptor>,
) -> Result<SpawnParallelGraphOutcome, String> {
    let phases = SPAWN_PARALLEL_PHASES;
    let label = format!("spawn_parallel_agents:{parent_session}");
    let parent_for_dispatch_session = parent_session.to_string();
    let progress_for_dispatch = progress_sink.clone();
    let definitions_for_dispatch = definitions.clone();
    let parent_for_dispatch = parent.clone();
    let parent_workspace_for_dispatch = parent_workspace_descriptor.clone();
    let parent_for_collect = parent_session.to_string();
    let progress_for_collect = progress_sink.clone();
    let parent_for_finalize = parent_session.to_string();
    let cancel_for_validate = cancel.clone();
    let cancel_for_dispatch = cancel.clone();
    let cancel_for_worker = cancel.clone();
    let cancel_for_collect = cancel.clone();
    let cancel_for_finalize = cancel.clone();
    let graph = GraphBuilder::<SpawnParallelState, SpawnParallelUpdate>::new()
        .set_reducer(ClosureStateReducer::new(
            |mut state: SpawnParallelState, update: SpawnParallelUpdate| {
                match update {
                    SpawnParallelUpdate::PhaseEntered(phase) => state.visited.push(phase),
                    SpawnParallelUpdate::Cancelled(phase) => {
                        state.visited.push(phase);
                        state.cancelled_phase.get_or_insert(phase);
                    }
                    SpawnParallelUpdate::Rejected(message) => state.rejection = Some(message),
                    SpawnParallelUpdate::Staged {
                        prepared,
                        immediate_results,
                    } => {
                        state.prepared = prepared;
                        state.immediate_results = immediate_results;
                    }
                    SpawnParallelUpdate::Fanned(results) => state.fanned_results = results,
                    SpawnParallelUpdate::Results(results) => state.results = results,
                    SpawnParallelUpdate::Collected(collected) => state.collected = Some(collected),
                }
                Ok(state)
            },
        ))
        .add_node(
            phases[0],
            move |state: SpawnParallelState, _ctx: NodeContext| {
                let cancel = cancel_for_validate.clone();
                async move {
                    if cancel.is_cancelled() {
                        tracing::debug!(
                            phase = "validate",
                            "[spawn_parallel_agents] graph_cancelled_at_boundary"
                        );
                        return Ok(NodeResult::Update(SpawnParallelUpdate::Cancelled(
                            "validate",
                        )));
                    }
                    if state.tasks.len() > state.max_parallel {
                        let message = format!(
                            "spawn_parallel_agents received {} tasks but max_parallel_tools is {}",
                            state.tasks.len(),
                            state.max_parallel
                        );
                        Ok(NodeResult::Update(SpawnParallelUpdate::Rejected(message)))
                    } else {
                        Ok(NodeResult::Update(SpawnParallelUpdate::PhaseEntered(
                            "validate",
                        )))
                    }
                }
            },
        )
        .add_node(
            phases[1],
            move |state: SpawnParallelState, _ctx: NodeContext| {
                let parent_session = parent_for_dispatch_session.clone();
                let progress_sink = progress_for_dispatch.clone();
                let definitions = definitions_for_dispatch.clone();
                let parent = parent_for_dispatch.clone();
                let parent_workspace_descriptor = parent_workspace_for_dispatch.clone();
                let cancel = cancel_for_dispatch.clone();
                async move {
                    if state.cancelled_phase.is_some() || state.rejection.is_some() {
                        return Ok(NodeResult::Update(SpawnParallelUpdate::PhaseEntered(
                            "dispatch",
                        )));
                    }
                    if cancel.is_cancelled() {
                        tracing::debug!(
                            phase = "dispatch",
                            "[spawn_parallel_agents] graph_cancelled_at_boundary"
                        );
                        return Ok(NodeResult::Update(SpawnParallelUpdate::Cancelled(
                            "dispatch",
                        )));
                    }
                    let (prepared, immediate_results) = stage_spawn_parallel_workers_from_defs(
                        &parent_session,
                        progress_sink.as_ref(),
                        state.tasks,
                        &definitions,
                        &parent,
                        state.action_root.as_deref(),
                        parent_workspace_descriptor.as_ref(),
                    )
                    .await;
                    Ok(NodeResult::Update(SpawnParallelUpdate::Staged {
                        prepared,
                        immediate_results,
                    }))
                }
            },
        )
        .add_node(
            phases[2],
            move |state: SpawnParallelState, _ctx: NodeContext| {
                let cancel = cancel_for_worker.clone();
                async move {
                    if state.cancelled_phase.is_some() || state.rejection.is_some() {
                        return Ok(NodeResult::Update(SpawnParallelUpdate::PhaseEntered(
                            "worker",
                        )));
                    }
                    if cancel.is_cancelled() {
                        tracing::debug!(
                            phase = "worker",
                            "[spawn_parallel_agents] graph_cancelled_at_boundary"
                        );
                        return Ok(NodeResult::Update(SpawnParallelUpdate::Cancelled("worker")));
                    }
                    let fanned =
                        match run_spawn_parallel_workers(state.prepared, state.action_root, cancel)
                            .await
                        {
                            Ok(fanned) => fanned,
                            Err(TinyAgentsError::Cancelled) => {
                                tracing::debug!(
                                    phase = "worker",
                                    "[spawn_parallel_agents] fanout_cancelled"
                                );
                                return Ok(NodeResult::Update(SpawnParallelUpdate::Cancelled(
                                    "worker",
                                )));
                            }
                            Err(err) => return Err(err),
                        };
                    Ok(NodeResult::Update(SpawnParallelUpdate::Fanned(fanned)))
                }
            },
        )
        .add_node(
            phases[3],
            move |state: SpawnParallelState, _ctx: NodeContext| {
                let parent_session = parent_for_collect.clone();
                let progress_sink = progress_for_collect.clone();
                let cancel = cancel_for_collect.clone();
                async move {
                    if state.cancelled_phase.is_some() || state.rejection.is_some() {
                        return Ok(NodeResult::Update(SpawnParallelUpdate::PhaseEntered(
                            "collect",
                        )));
                    }
                    if cancel.is_cancelled() {
                        tracing::debug!(
                            phase = "collect",
                            "[spawn_parallel_agents] graph_cancelled_at_boundary"
                        );
                        return Ok(NodeResult::Update(SpawnParallelUpdate::Cancelled(
                            "collect",
                        )));
                    }
                    let mut results = state.immediate_results;
                    for result in state.fanned_results {
                        project_spawn_parallel_result(
                            &parent_session,
                            progress_sink.as_ref(),
                            &result,
                        )
                        .await;
                        results.push(result);
                    }
                    Ok(NodeResult::Update(SpawnParallelUpdate::Results(results)))
                }
            },
        )
        .add_node(
            phases[4],
            move |state: SpawnParallelState, _ctx: NodeContext| {
                let parent_session = parent_for_finalize.clone();
                let cancel = cancel_for_finalize.clone();
                async move {
                    if state.cancelled_phase.is_some() {
                        return Ok(NodeResult::Update(SpawnParallelUpdate::PhaseEntered(
                            "finalize",
                        )));
                    }
                    if cancel.is_cancelled() {
                        tracing::debug!(
                            phase = "finalize",
                            "[spawn_parallel_agents] graph_cancelled_at_boundary"
                        );
                        return Ok(NodeResult::Update(SpawnParallelUpdate::Cancelled(
                            "finalize",
                        )));
                    }
                    if let Some(message) = state.rejection {
                        return Ok(NodeResult::Update(SpawnParallelUpdate::Rejected(message)));
                    }
                    let collected = collect_spawn_parallel_results(&parent_session, state.results);
                    Ok(NodeResult::Update(SpawnParallelUpdate::Collected(
                        collected,
                    )))
                }
            },
        )
        .add_edge(phases[0], phases[1])
        .add_edge(phases[1], phases[2])
        .add_edge(phases[2], phases[3])
        .add_edge(phases[3], phases[4])
        .set_entry(phases[0])
        .set_finish(phases[4])
        .compile()
        .map_err(|e| format!("spawn_parallel_agents graph compile failed: {e}"))?
        .with_event_sink(Arc::new(
            crate::openhuman::agent::tinyagents::observability::GraphTracingSink::new(label),
        ))
        // Adapter-first landing of the crate-native per-node RetryPolicy
        // (tinyagents 1.5.0 `CompiledGraph::with_node_retry`). Conservative:
        // `max_attempts(1)` preserves today's single-attempt phase semantics
        // exactly (no bespoke retry glue existed on these phases) and backoff
        // sleeping stays off (the default). Per-worker fanout resilience is
        // owned inside the worker/collect phases, not the phase-graph node loop;
        // this wires the crate seam so a future slice can raise the attempt cap
        // without re-plumbing.
        .with_node_retry(RetryPolicy::default().with_max_attempts(1));

    tracing::debug!(
        parent_session = %parent_session,
        "[spawn_parallel_agents] running graph fanout"
    );
    let execution = graph
        .run(SpawnParallelState::for_execution(
            tasks,
            max_parallel,
            action_root,
        ))
        .await
        .map_err(|e| format!("spawn_parallel_agents graph run failed: {e}"))?;

    if let Some(phase) = execution.state.cancelled_phase {
        return Ok(SpawnParallelGraphOutcome::Cancelled(format!(
            "spawn_parallel_agents cancelled at {phase}"
        )));
    }
    if let Some(message) = execution.state.rejection {
        return Ok(SpawnParallelGraphOutcome::Rejected(message));
    }
    execution
        .state
        .collected
        .map(SpawnParallelGraphOutcome::Collected)
        .ok_or_else(|| "spawn_parallel_agents graph finished without collected results".to_string())
}

/// Structure-only topology of the `spawn_parallel_agents` graph.
pub(crate) fn spawn_parallel_graph_topology() -> Result<GraphTopology, String> {
    Ok(build_spawn_parallel_graph()?.topology())
}
