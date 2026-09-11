
fn spawn_parallel_lineage(
    parent_session: &str,
    session_parent_prefix: Option<&str>,
    task_id: &str,
) -> ParallelAgentLineage {
    let root_session = session_parent_prefix
        .and_then(|prefix| prefix.split("__").next())
        .filter(|root| !root.is_empty())
        .unwrap_or(parent_session);
    ParallelAgentLineage {
        parent_session: parent_session.to_string(),
        root_session: root_session.to_string(),
        child_task_id: task_id.to_string(),
    }
}

async fn stage_spawn_parallel_workers_from_defs(
    parent_session: &str,
    progress_sink: Option<&Sender<AgentProgress>>,
    tasks: Vec<ParallelAgentTask>,
    definitions: &HashMap<String, AgentDefinition>,
    parent: &ParentExecutionContext,
    action_root: Option<&Path>,
    parent_workspace_descriptor: Option<&WorkspaceDescriptor>,
) -> (Vec<SpawnParallelWorker>, Vec<ParallelAgentResult>) {
    let mut immediate_results = Vec::new();
    let mut prepared = Vec::new();

    for preflight in prepare_spawn_parallel_tasks_from_defs(tasks, definitions, parent) {
        let (definition, prompt, task, task_id, dispatch_mode) = match preflight {
            SpawnParallelTaskPreflight::Rejected(rejection) => {
                match rejection.kind {
                    ParallelTaskRejectionKind::MissingAgentOrPrompt => {
                        tracing::debug!(
                            parent_session = %parent_session,
                            task_id = %rejection.task_id,
                            agent_id = %rejection.agent_id,
                            "[spawn_parallel_agents] invalid_task_missing_agent_or_prompt"
                        );
                    }
                    ParallelTaskRejectionKind::UnknownAgent => {
                        tracing::debug!(
                            parent_session = %parent_session,
                            task_id = %rejection.task_id,
                            agent_id = %rejection.agent_id,
                            "[spawn_parallel_agents] invalid_task_unknown_agent"
                        );
                    }
                    ParallelTaskRejectionKind::OutsideAllowlist => {
                        tracing::warn!(
                            parent_session = %parent_session,
                            parent_agent = %parent.agent_definition_id,
                            task_id = %rejection.task_id,
                            agent_id = %rejection.agent_id,
                            allowed = ?parent.allowed_subagent_ids,
                            "[spawn_parallel_agents] rejected_task_outside_subagent_allowlist"
                        );
                    }
                    ParallelTaskRejectionKind::MissingToolkit => {
                        tracing::debug!(
                            parent_session = %parent_session,
                            task_id = %rejection.task_id,
                            agent_id = %rejection.agent_id,
                            "[spawn_parallel_agents] invalid_task_missing_toolkit"
                        );
                    }
                    ParallelTaskRejectionKind::RequiresIsolation => {
                        tracing::warn!(
                            parent_session = %parent_session,
                            task_id = %rejection.task_id,
                            agent_id = %rejection.agent_id,
                            ownership = rejection.ownership.as_deref().unwrap_or(""),
                            "[spawn_parallel_agents] rejected_shared_workspace_write_capable_task"
                        );
                    }
                }
                let lineage = spawn_parallel_lineage(
                    parent_session,
                    parent.session_parent_prefix.as_deref(),
                    &rejection.task_id,
                );
                immediate_results.push(ParallelAgentResult {
                    task_id: rejection.task_id,
                    agent_id: rejection.agent_id,
                    lineage,
                    success: false,
                    output: None,
                    error: Some(rejection.error),
                    ownership: rejection.ownership,
                    elapsed_ms: 0,
                    iterations: 0,
                    stale_parent_reads: Vec::new(),
                    worktree_path: None,
                    changed_files: Vec::new(),
                    dirty_status: None,
                });
                continue;
            }
            SpawnParallelTaskPreflight::Prepared(prepared_task) => (
                prepared_task.definition,
                prepared_task.prompt,
                prepared_task.task,
                prepared_task.task_id,
                prepared_task.dispatch_mode,
            ),
        };
        project_spawn_parallel_spawned(
            parent_session,
            progress_sink,
            &definition,
            &task_id,
            &prompt,
            task.ownership
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_some(),
        )
        .await;
        let workspace_descriptor = match create_spawn_parallel_worktree(
            parent_session,
            action_root,
            &task_id,
            &definition,
            &task,
            parent.session_parent_prefix.as_deref(),
        )
        .await
        {
            Ok(descriptor) => descriptor,
            Err(result) => {
                immediate_results.push(result);
                continue;
            }
        };
        let worktree_path = workspace_descriptor
            .as_ref()
            .map(|descriptor| descriptor.root.clone());
        let worker_workspace_descriptor = workspace_descriptor
            .clone()
            .or_else(|| parent_workspace_descriptor.cloned());
        let lineage = spawn_parallel_lineage(
            parent_session,
            parent.session_parent_prefix.as_deref(),
            &task_id,
        );
        prepared.push(SpawnParallelWorker {
            definition,
            prompt,
            task,
            task_id,
            lineage,
            worktree_path,
            workspace_descriptor: worker_workspace_descriptor,
            dispatch_mode,
        });
    }

    tracing::debug!(
        parent_session = %parent_session,
        prepared_count = prepared.len(),
        immediate_count = immediate_results.len(),
        serial_write_count = prepared
            .iter()
            .filter(|worker| matches!(
                worker.dispatch_mode,
                WorkerDispatchMode::SerialSharedWorkspaceWrite
            ))
            .count(),
        "[spawn_parallel_agents] prepared_tasks"
    );
    (prepared, immediate_results)
}

pub(super) async fn run_spawn_parallel_graph(
    args: serde_json::Value,
) -> Result<SpawnParallelGraphOutcome, String> {
    run_spawn_parallel_graph_with_workspace(args, None).await
}

pub(super) async fn run_spawn_parallel_graph_with_workspace(
    args: serde_json::Value,
    parent_workspace_descriptor: Option<WorkspaceDescriptor>,
) -> Result<SpawnParallelGraphOutcome, String> {
    run_spawn_parallel_graph_with_cancellation_and_workspace(
        args,
        CancellationToken::new(),
        parent_workspace_descriptor,
    )
    .await
}

pub(super) async fn run_spawn_parallel_graph_with_cancellation(
    args: serde_json::Value,
    cancel: CancellationToken,
) -> Result<SpawnParallelGraphOutcome, String> {
    run_spawn_parallel_graph_with_cancellation_and_workspace(args, cancel, None).await
}

pub(super) async fn run_spawn_parallel_graph_with_cancellation_and_workspace(
    args: serde_json::Value,
    cancel: CancellationToken,
    parent_workspace_descriptor: Option<WorkspaceDescriptor>,
) -> Result<SpawnParallelGraphOutcome, String> {
    let tasks = match validate_spawn_parallel_tool_request(&args, None) {
        Ok(tasks) => tasks,
        Err(err) => return Ok(SpawnParallelGraphOutcome::InvalidRequest(err)),
    };

    let parent = match current_parent() {
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
    let registry = match AgentDefinitionRegistry::global() {
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
    let definitions = snapshot_agent_definitions(registry);
    let outcome = run_spawn_parallel_execution_graph(
        &parent_session,
        progress_sink,
        tasks,
        max_parallel,
        definitions,
        parent,
        action_root,
        cancel,
        parent_workspace_descriptor,
    )
    .await?;
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
        SpawnParallelGraphOutcome::InvalidRequest(_) => {
            tracing::debug!(
                parent_session = %parent_session,
                "[spawn_parallel_agents] invalid_request_after_graph_run"
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
    match crate::openhuman::config::Config::load_or_init().await {
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

#[derive(Clone)]
pub(super) struct SpawnParallelCollected {
    pub(super) results: Vec<ParallelAgentResult>,
    pub(super) failures: usize,
    pub(super) overlap_warnings: Vec<serde_json::Value>,
}

pub(super) enum SpawnParallelGraphOutcome {
    Collected(SpawnParallelCollected),
    InvalidRequest(SpawnParallelTaskValidationError),
    Rejected(String),
    Cancelled(String),
}

impl SpawnParallelCollected {
    pub(super) fn total(&self) -> usize {
        self.results.len()
    }

    pub(super) fn succeeded(&self) -> usize {
        self.results.len().saturating_sub(self.failures)
    }
}

fn collect_spawn_parallel_results(
    parent_session: &str,
    mut results: Vec<ParallelAgentResult>,
) -> SpawnParallelCollected {
    annotate_stale_parent_reads(&mut results);
    let overlap_warnings = overlap_warnings_for_results(parent_session, &results);
    let failures = results.iter().filter(|r| !r.success).count();
    SpawnParallelCollected {
        results,
        failures,
        overlap_warnings,
    }
}

pub(super) fn format_spawn_parallel_success(collected: &SpawnParallelCollected) -> String {
    serde_json::to_string_pretty(&json!({
        "parallel_agents": {
            "total": collected.total(),
            "succeeded": collected.succeeded(),
            "failed": collected.failures,
            "results": collected.results,
            "overlap_warnings": collected.overlap_warnings,
        }
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

async fn project_spawn_parallel_spawned(
    parent_session: &str,
    progress_sink: Option<&Sender<AgentProgress>>,
    definition: &AgentDefinition,
    task_id: &str,
    prompt: &str,
    has_ownership: bool,
) {
    let prompt_chars = prompt.chars().count();
    tracing::debug!(
        parent_session = %parent_session,
        task_id = %task_id,
        agent_id = %definition.id,
        prompt_chars,
        has_ownership,
        "[spawn_parallel_agents] publishing_subagent_spawned"
    );
    crate::openhuman::agent::orchestration::subagent_events::publish_subagent_spawned(
        parent_session.to_string(),
        definition.id.clone(),
        "typed".to_string(),
        task_id.to_string(),
        prompt_chars,
    );
    if let Some(tx) = progress_sink {
        if let Err(err) = tx
            .send(AgentProgress::SubagentSpawned {
                agent_id: definition.id.clone(),
                task_id: task_id.to_string(),
                mode: "typed".to_string(),
                dedicated_thread: false,
                prompt_chars,
                prompt: prompt.to_string(),
                worker_thread_id: None,
                display_name: Some(definition.display_name().to_string()),
            })
            .await
        {
            tracing::debug!(
                parent_session = %parent_session,
                task_id = %task_id,
                agent_id = %definition.id,
                error = %err,
                "[spawn_parallel_agents] progress_send_failed spawned"
            );
        }
    }
}

async fn project_spawn_parallel_result(
    parent_session: &str,
    progress_sink: Option<&Sender<AgentProgress>>,
    result: &ParallelAgentResult,
) {
    match result {
        ParallelAgentResult {
            success: true,
            agent_id,
            task_id,
            elapsed_ms,
            iterations,
            output,
            worktree_path,
            changed_files,
            dirty_status,
            ..
        } => {
            tracing::debug!(
                parent_session = %parent_session,
                task_id = %task_id,
                agent_id = %agent_id,
                elapsed_ms = *elapsed_ms,
                iterations = *iterations,
                "[spawn_parallel_agents] publishing_subagent_completed"
            );
            crate::openhuman::agent::orchestration::subagent_events::publish_subagent_completed(
                parent_session.to_string(),
                task_id.clone(),
                agent_id.clone(),
                *elapsed_ms,
                output.as_ref().map(|s| s.chars().count()).unwrap_or(0),
                *iterations as usize,
            );
            if let Some(tx) = progress_sink {
                if let Err(err) = tx
                    .send(AgentProgress::SubagentCompleted {
                        agent_id: agent_id.clone(),
                        task_id: task_id.clone(),
                        elapsed_ms: *elapsed_ms,
                        iterations: *iterations,
                        output_chars: output.as_ref().map(|s| s.chars().count()).unwrap_or(0),
                        output: output.clone().unwrap_or_default(),
                        worktree_path: worktree_path.clone(),
                        changed_files: changed_files.clone(),
                        dirty_status: *dirty_status,
                    })
                    .await
                {
                    tracing::debug!(
                        parent_session = %parent_session,
                        task_id = %task_id,
                        agent_id = %agent_id,
                        error = %err,
                        "[spawn_parallel_agents] progress_send_failed completed"
                    );
                }
            }
        }
        ParallelAgentResult {
            success: false,
            agent_id,
            task_id,
            error,
            ..
        } => {
            let message = error
                .clone()
                .unwrap_or_else(|| "unknown failure".to_string());
            tracing::debug!(
                parent_session = %parent_session,
                task_id = %task_id,
                agent_id = %agent_id,
                error = %message,
                "[spawn_parallel_agents] publishing_subagent_failed"
            );
            crate::openhuman::agent::orchestration::subagent_events::publish_subagent_failed(
                parent_session.to_string(),
                task_id.clone(),
                agent_id.clone(),
                message.clone(),
            );
            if let Some(tx) = progress_sink {
                if let Err(err) = tx
                    .send(AgentProgress::SubagentFailed {
                        agent_id: agent_id.clone(),
                        task_id: task_id.clone(),
                        error: message,
                    })
                    .await
                {
                    tracing::debug!(
                        parent_session = %parent_session,
                        task_id = %task_id,
                        agent_id = %agent_id,
                        error = %err,
                        "[spawn_parallel_agents] progress_send_failed failed"
                    );
                }
            }
        }
    }
}

fn annotate_stale_parent_reads(results: &mut [ParallelAgentResult]) {
    if let Some(parent_agent_id) = file_state::current_file_state_agent_id() {
        let child_ids: Vec<String> = results.iter().map(|r| r.task_id.clone()).collect();
        let stale = file_state::parent_stale_files(&parent_agent_id, &child_ids);
        if !stale.is_empty() {
            let stale_strings: Vec<String> =
                stale.iter().map(|p| p.display().to_string()).collect();
            tracing::debug!(
                parent = %parent_agent_id,
                stale_count = stale.len(),
                "[file_state] parent reads stale after child writes"
            );
            for result in results {
                result.stale_parent_reads = stale_strings.clone();
            }
        }
    }
}

fn overlap_warnings_for_results(
    parent_session: &str,
    results: &[ParallelAgentResult],
) -> Vec<serde_json::Value> {
    let per_worker: Vec<(String, Vec<PathBuf>)> = results
        .iter()
        .filter(|r| !r.changed_files.is_empty())
        .map(|r| {
            (
                r.task_id.clone(),
                r.changed_files.iter().map(PathBuf::from).collect(),
            )
        })
        .collect();
    let overlaps = crate::openhuman::agent::orchestration::worktree::detect_overlaps(&per_worker);
    let overlap_warnings: Vec<serde_json::Value> = overlaps
        .iter()
        .map(|(file, workers)| {
            json!({
                "file": file.to_string_lossy(),
                "workers": workers,
            })
        })
        .collect();
    if !overlap_warnings.is_empty() {
        tracing::warn!(
            parent_session = %parent_session,
            overlap_count = overlap_warnings.len(),
            "[spawn_parallel_agents] detected overlapping changed files across workers"
        );
    }
    overlap_warnings
}

async fn run_spawn_parallel_workers(
    prepared: Vec<SpawnParallelWorker>,
    action_root: Option<PathBuf>,
    cancel: CancellationToken,
) -> tinyagents_harness::Result<Vec<ParallelAgentResult>> {
    let n = prepared.len();
    let serial_write_count = prepared
        .iter()
        .filter(|worker| {
            matches!(
                worker.dispatch_mode,
                WorkerDispatchMode::SerialSharedWorkspaceWrite
            )
        })
        .count();
    if serial_write_count > 0 {
        tracing::debug!(
            target: "orchestration",
            workers = n,
            serial_write_count,
            "[orchestration] running serial fallback for shared-workspace write fan-out"
        );
        let mut results = Vec::with_capacity(n);
        for worker in prepared {
            if cancel.is_cancelled() {
                tracing::debug!(
                    target: "orchestration",
                    "[orchestration] spawn_parallel serial fan-out cancelled before next worker"
                );
                return Err(TinyAgentsError::Cancelled);
            }
            results.push(run_one_parallel_task(worker, action_root.clone()).await);
        }
        return Ok(results);
    }

    let max_concurrency = prepared.len().max(1);
    let action_root_for_workers = action_root.clone();
    tracing::debug!(
        target: "orchestration",
        workers = n,
        max_concurrency,
        "[orchestration] running parallel fan-out on tinyagents map_reduce (spawn_parallel_agents)"
    );
    let options = ParallelOptions::default()
        .with_max_concurrency(max_concurrency)
        .with_failure_policy(FailurePolicy::CollectAll)
        .with_cancellation(cancel);
    let outcome = map_reduce(prepared, options, move |_i, worker| {
        let repo_root = action_root_for_workers.clone();
        async move { Ok(run_one_parallel_task(worker, repo_root).await) }
    })
    .await?;

    let mut results = Vec::with_capacity(n);
    for item in outcome.outcomes {
        match item.result {
            Ok(value) => results.push(value),
            Err(err) => {
                return Err(TinyAgentsError::Graph(format!(
                    "spawn_parallel_agents fan-out: worker {} failed: {err}",
                    item.index
                )));
            }
        }
    }
    if results.len() != n {
        return Err(TinyAgentsError::Graph(format!(
            "spawn_parallel_agents fan-out: expected {n} result(s), got {}",
            results.len()
        )));
    }
    Ok(results)
}
