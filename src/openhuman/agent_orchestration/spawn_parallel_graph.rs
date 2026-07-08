//! Structure-only graph scaffold for `spawn_parallel_agents`.
//!
//! The tool wrapper still owns `ToolResult` translation. This module owns
//! request parsing, parent-context validation, graph-side request validation,
//! worktree preflight, progress/event projection, worker fanout, final JSON
//! formatting, and the topology surface from
//! `docs/tinyagents-full-migration-plan/08-orchestration/
//! 02-spawn-parallel-graph.md`.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use tinyagents::graph::export::GraphTopology;
use tinyagents::graph::parallel::{map_reduce, FailurePolicy, ParallelOptions};
use tinyagents::graph::{
    ClosureStateReducer, CompiledGraph, GraphBuilder, NodeContext, NodeResult,
};
use tinyagents::harness::retry::RetryPolicy;
use tinyagents::harness::workspace::{WorkspaceDescriptor, WorkspaceIsolation};
use tinyagents::{CancellationToken, TinyAgentsError};

use crate::openhuman::agent::harness::definition::{
    AgentDefinition, AgentDefinitionRegistry, SandboxMode, ToolScope,
};
use crate::openhuman::agent::harness::fork_context::{current_parent, ParentExecutionContext};
use crate::openhuman::agent::harness::subagent_runner::{run_subagent, SubagentRunOptions};
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::agent_orchestration::worktree::{self, BaseRef};
use crate::openhuman::file_state;
use crate::openhuman::tools::PermissionLevel;
use serde::Serialize;
use serde_json::json;
use tokio::sync::mpsc::Sender;

/// One requested worker in a `spawn_parallel_agents` call.
#[derive(Debug, Clone, serde::Deserialize)]
pub(super) struct ParallelAgentTask {
    pub(super) agent_id: String,
    pub(super) prompt: String,
    #[serde(default)]
    pub(super) context: Option<String>,
    #[serde(default)]
    pub(super) toolkit: Option<String>,
    #[serde(default)]
    pub(super) ownership: Option<String>,
    /// File-isolation strategy for this worker: `"none"` (default) or
    /// `"worktree"` (dedicated git worktree checkout).
    #[serde(default)]
    pub(super) isolation: Option<String>,
    /// Worktree base ref: `"head"` (default) or `"fresh"`.
    #[serde(default)]
    pub(super) base_ref: Option<String>,
}

/// Decode and validate the request batch before the live worker fanout.
///
/// This is the first real `validate`-node responsibility moved out of the tool
/// wrapper. Effectful worktree creation now runs in the graph path after the
/// action root and worker definitions are resolved.
fn validate_spawn_parallel_tasks(
    args: &serde_json::Value,
    max_parallel: Option<usize>,
) -> Result<Vec<ParallelAgentTask>, String> {
    let tasks_value = args
        .get("tasks")
        .cloned()
        .ok_or_else(|| "Missing 'tasks' parameter".to_string())?;
    let tasks: Vec<ParallelAgentTask> =
        serde_json::from_value(tasks_value).map_err(|e| format!("Invalid tasks array: {e}"))?;

    if tasks.len() < 2 {
        return Err("spawn_parallel_agents requires at least two tasks".to_string());
    }
    if let Some(max_parallel) = max_parallel {
        if tasks.len() > max_parallel {
            return Err(format!(
                "spawn_parallel_agents received {} tasks but max_parallel_tools is {}",
                tasks.len(),
                max_parallel
            ));
        }
    }

    Ok(tasks)
}

pub(super) enum SpawnParallelTaskValidationError {
    MissingTasks(String),
    InvalidTasks(String),
    Rejected(String),
}

fn validate_spawn_parallel_tool_request(
    args: &serde_json::Value,
    max_parallel: Option<usize>,
) -> Result<Vec<ParallelAgentTask>, SpawnParallelTaskValidationError> {
    validate_spawn_parallel_tasks(args, max_parallel).map_err(|message| {
        if message == "Missing 'tasks' parameter" {
            SpawnParallelTaskValidationError::MissingTasks(message)
        } else if message.starts_with("Invalid tasks array:") {
            SpawnParallelTaskValidationError::InvalidTasks(message)
        } else {
            SpawnParallelTaskValidationError::Rejected(message)
        }
    })
}

/// Prepared worker ready for the live dispatch/worker phases.
pub(crate) struct PreparedParallelTask {
    definition: AgentDefinition,
    prompt: String,
    task: ParallelAgentTask,
    task_id: String,
    dispatch_mode: WorkerDispatchMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParallelTaskRejectionKind {
    MissingAgentOrPrompt,
    UnknownAgent,
    OutsideAllowlist,
    MissingToolkit,
    RequiresIsolation,
}

pub(crate) struct ParallelTaskRejection {
    pub(crate) task_id: String,
    pub(crate) agent_id: String,
    pub(crate) error: String,
    pub(crate) ownership: Option<String>,
    pub(crate) kind: ParallelTaskRejectionKind,
}

pub(crate) enum SpawnParallelTaskPreflight {
    Prepared(PreparedParallelTask),
    Rejected(ParallelTaskRejection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerDispatchMode {
    Parallel,
    SerialSharedWorkspaceWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParallelWorktreeRequest {
    SharedWorkspace,
    Isolated { base_ref: BaseRef },
}

fn worktree_request_for_task(task: &ParallelAgentTask) -> ParallelWorktreeRequest {
    let isolated = task
        .isolation
        .as_deref()
        .map(str::trim)
        .map(|s| s.eq_ignore_ascii_case("worktree"))
        .unwrap_or(false);
    if isolated {
        ParallelWorktreeRequest::Isolated {
            base_ref: BaseRef::parse(task.base_ref.as_deref()),
        }
    } else {
        ParallelWorktreeRequest::SharedWorkspace
    }
}

fn disallowed_tool_matches(disallowed: &[String], name: &str) -> bool {
    disallowed.iter().any(|entry| {
        if let Some(prefix) = entry.strip_suffix('*') {
            name.starts_with(prefix)
        } else {
            entry == name
        }
    })
}

fn definition_visible_tool_permissions(
    definition: &AgentDefinition,
    parent: &ParentExecutionContext,
) -> Vec<(String, PermissionLevel)> {
    let skill_prefix = definition
        .skill_filter
        .as_ref()
        .map(|skill| format!("{skill}__"));
    parent
        .all_tools
        .iter()
        .filter_map(|tool| {
            let name = tool.name();
            if disallowed_tool_matches(&definition.disallowed_tools, name) {
                return None;
            }
            if let Some(prefix) = skill_prefix.as_deref() {
                if !name.starts_with(prefix) {
                    return None;
                }
            }
            let allowed = match &definition.tools {
                ToolScope::Wildcard => true,
                ToolScope::Named(names) => {
                    names.iter().any(|allowed| allowed == name)
                        || definition.extra_tools.iter().any(|extra| extra == name)
                        || (crate::openhuman::tokenjuice::is_recovery_tool(name)
                            && !names.is_empty())
                }
            };
            allowed.then(|| (name.to_string(), tool.permission_level()))
        })
        .collect()
}

fn shared_workspace_write_capable_tools(
    definition: &AgentDefinition,
    parent: &ParentExecutionContext,
) -> Vec<String> {
    let mut write_capable_tools = definition_visible_tool_permissions(definition, parent)
        .into_iter()
        .filter(|(_, level)| *level > PermissionLevel::ReadOnly)
        .map(|(name, level)| format!("{name}:{level}"))
        .collect::<Vec<_>>();
    write_capable_tools.sort();
    write_capable_tools.dedup();
    write_capable_tools
}

fn shared_workspace_write_preview(write_capable_tools: &[String]) -> String {
    let preview = write_capable_tools
        .iter()
        .take(6)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let suffix = if write_capable_tools.len() > 6 {
        format!(", +{} more", write_capable_tools.len() - 6)
    } else {
        String::new()
    };
    format!("{preview}{suffix}")
}

fn ownership_file_paths(ownership: Option<&str>) -> Result<Vec<PathBuf>, String> {
    let Some(ownership) = ownership.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(Vec::new());
    };
    let Some(rest) = ownership.strip_prefix("files:") else {
        return Ok(Vec::new());
    };
    let mut paths = Vec::new();
    for raw in rest.split([',', '\n']) {
        let trimmed = raw
            .trim()
            .trim_start_matches(|c: char| c == '-' || c == '*')
            .trim();
        if trimmed.is_empty() {
            continue;
        }
        let path = PathBuf::from(trimmed);
        if path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir | std::path::Component::Prefix(_)
                )
            })
        {
            return Err(format!(
                "ownership path '{trimmed}' must be a relative file path under the workspace"
            ));
        }
        paths.push(path);
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

fn shared_workspace_write_claim(
    task: &ParallelAgentTask,
    definition: &AgentDefinition,
    parent: &ParentExecutionContext,
) -> Result<Option<Vec<PathBuf>>, String> {
    if matches!(
        worktree_request_for_task(task),
        ParallelWorktreeRequest::Isolated { .. }
    ) {
        return Ok(None);
    }
    if matches!(definition.sandbox_mode, SandboxMode::ReadOnly) {
        return Ok(None);
    }
    let write_capable_tools = shared_workspace_write_capable_tools(definition, parent);
    if write_capable_tools.is_empty() {
        return Ok(None);
    }
    let paths = ownership_file_paths(task.ownership.as_deref())?;
    if paths.is_empty() {
        return Err(format!(
            "agent '{}' can use write/execute tools in the shared workspace ({}); \
             set isolation=\"worktree\" for edit-capable parallel workers, use a read-only agent, \
             or provide disjoint files: ownership for serial fallback",
            definition.id,
            shared_workspace_write_preview(&write_capable_tools)
        ));
    }
    Ok(Some(paths))
}

async fn create_spawn_parallel_worktree(
    parent_session: &str,
    action_root: Option<&Path>,
    task_id: &str,
    definition: &AgentDefinition,
    task: &ParallelAgentTask,
    session_parent_prefix: Option<&str>,
) -> Result<Option<WorkspaceDescriptor>, ParallelAgentResult> {
    match worktree_request_for_task(task) {
        ParallelWorktreeRequest::SharedWorkspace => Ok(None),
        ParallelWorktreeRequest::Isolated { base_ref } => match action_root {
            Some(repo_root) => {
                let sandbox = match definition.sandbox_mode {
                    SandboxMode::Sandboxed => tinyagents::harness::tool::SandboxMode::Required,
                    SandboxMode::None | SandboxMode::ReadOnly => {
                        tinyagents::harness::tool::SandboxMode::Inherit
                    }
                };
                let isolation = worktree::GitWorktreeIsolation::new(repo_root)
                    .with_base_ref(base_ref)
                    .with_sandbox(sandbox);
                match isolation.prepare(task_id, Some(&definition.id)).await {
                    Ok(descriptor) => {
                        tracing::debug!(
                            parent_session = %parent_session,
                            task_id = %task_id,
                            worktree = %descriptor.root.display(),
                            policy_id = %descriptor.policy_id,
                            base_ref = base_ref.as_str(),
                            "[spawn_parallel_agents] prepared isolated workspace descriptor"
                        );
                        Ok(Some(descriptor))
                    }
                    Err(err) => {
                        tracing::warn!(
                            parent_session = %parent_session,
                            task_id = %task_id,
                            error = %err,
                            "[spawn_parallel_agents] workspace_prepare_failed"
                        );
                        Err(ParallelAgentResult {
                            task_id: task_id.to_string(),
                            agent_id: definition.id.clone(),
                            lineage: spawn_parallel_lineage(
                                parent_session,
                                session_parent_prefix,
                                task_id,
                            ),
                            success: false,
                            output: None,
                            error: Some(format!("worktree isolation failed: {err}")),
                            ownership: task.ownership.clone(),
                            elapsed_ms: 0,
                            iterations: 0,
                            stale_parent_reads: Vec::new(),
                            worktree_path: None,
                            changed_files: Vec::new(),
                            dirty_status: None,
                        })
                    }
                }
            }
            None => {
                tracing::warn!(
                    parent_session = %parent_session,
                    task_id = %task_id,
                    "[spawn_parallel_agents] worktree_requested_but_no_action_dir"
                );
                Err(ParallelAgentResult {
                    task_id: task_id.to_string(),
                    agent_id: definition.id.clone(),
                    lineage: spawn_parallel_lineage(parent_session, session_parent_prefix, task_id),
                    success: false,
                    output: None,
                    error: Some(
                        "worktree isolation requested but action_dir is unavailable".to_string(),
                    ),
                    ownership: task.ownership.clone(),
                    elapsed_ms: 0,
                    iterations: 0,
                    stale_parent_reads: Vec::new(),
                    worktree_path: None,
                    changed_files: Vec::new(),
                    dirty_status: None,
                })
            }
        },
    }
}

fn snapshot_agent_definitions(
    registry: &AgentDefinitionRegistry,
) -> HashMap<String, AgentDefinition> {
    registry
        .list()
        .into_iter()
        .map(|definition| (definition.id.clone(), definition.clone()))
        .collect()
}

pub(crate) fn prepare_spawn_parallel_tasks_from_defs(
    tasks: Vec<ParallelAgentTask>,
    definitions: &HashMap<String, AgentDefinition>,
    parent: &ParentExecutionContext,
) -> Vec<SpawnParallelTaskPreflight> {
    let mut serial_write_claims: Vec<(PathBuf, String)> = Vec::new();
    tasks
        .into_iter()
        .map(|task| {
            let agent_id = task.agent_id.trim().to_string();
            let prompt = task.prompt.trim().to_string();
            let task_id = format!("sub-{}", uuid::Uuid::new_v4());

            if agent_id.is_empty() || prompt.is_empty() {
                return SpawnParallelTaskPreflight::Rejected(ParallelTaskRejection {
                    task_id,
                    agent_id,
                    error: "agent_id and prompt are required".to_string(),
                    ownership: task.ownership,
                    kind: ParallelTaskRejectionKind::MissingAgentOrPrompt,
                });
            }

            let Some(definition) = definitions.get(&agent_id).cloned() else {
                return SpawnParallelTaskPreflight::Rejected(ParallelTaskRejection {
                    task_id,
                    agent_id: agent_id.clone(),
                    error: format!("unknown agent_id '{agent_id}'"),
                    ownership: task.ownership,
                    kind: ParallelTaskRejectionKind::UnknownAgent,
                });
            };

            if !parent.allowed_subagent_ids.contains(&definition.id) {
                return SpawnParallelTaskPreflight::Rejected(ParallelTaskRejection {
                    task_id,
                    agent_id: definition.id.clone(),
                    error: format!(
                        "agent '{}' is not in parent agent '{}' subagents.allowlist",
                        definition.id, parent.agent_definition_id
                    ),
                    ownership: task.ownership,
                    kind: ParallelTaskRejectionKind::OutsideAllowlist,
                });
            }

            if definition.id == "integrations_agent"
                && task
                    .toolkit
                    .as_ref()
                    .map(|s| s.trim().is_empty())
                    .unwrap_or(true)
            {
                return SpawnParallelTaskPreflight::Rejected(ParallelTaskRejection {
                    task_id,
                    agent_id,
                    error: "integrations_agent requires toolkit".to_string(),
                    ownership: task.ownership,
                    kind: ParallelTaskRejectionKind::MissingToolkit,
                });
            }

            let dispatch_mode =
                match shared_workspace_write_claim(&task, &definition, parent) {
                    Ok(Some(paths)) => {
                        if let Some((overlap_path, overlap_task)) =
                            paths.iter().find_map(|path| {
                                serial_write_claims
                                    .iter()
                                    .find(|(claimed, _)| paths_overlap(path, claimed))
                                    .map(|(claimed, task_id)| (claimed.clone(), task_id.clone()))
                            })
                        {
                            return SpawnParallelTaskPreflight::Rejected(
                                ParallelTaskRejection {
                                    task_id,
                                    agent_id: definition.id.clone(),
                                    error: format!(
                                        "agent '{}' requested shared-workspace write access to '{}' but it overlaps with serial worker {overlap_task}; set isolation=\"worktree\" or use disjoint files: ownership",
                                        definition.id,
                                        overlap_path.display()
                                    ),
                                    ownership: task.ownership,
                                    kind: ParallelTaskRejectionKind::RequiresIsolation,
                                },
                            );
                        }
                        for path in paths {
                            serial_write_claims.push((path, task_id.clone()));
                        }
                        WorkerDispatchMode::SerialSharedWorkspaceWrite
                    }
                    Ok(None) => WorkerDispatchMode::Parallel,
                    Err(error) => {
                        return SpawnParallelTaskPreflight::Rejected(ParallelTaskRejection {
                            task_id,
                            agent_id: definition.id.clone(),
                            error,
                            ownership: task.ownership,
                            kind: ParallelTaskRejectionKind::RequiresIsolation,
                        });
                    }
                };

            let prompt = with_ownership_boundary(&prompt, task.ownership.as_deref());
            SpawnParallelTaskPreflight::Prepared(PreparedParallelTask {
                definition,
                prompt,
                task,
                task_id,
                dispatch_mode,
            })
        })
        .collect()
}

pub(super) fn with_ownership_boundary(prompt: &str, ownership: Option<&str>) -> String {
    match ownership.map(str::trim).filter(|s| !s.is_empty()) {
        Some(boundary) => format!(
            "[Ownership Boundary]\n{boundary}\n\n[Task]\n{prompt}\n\nDo not work outside the ownership boundary unless the parent explicitly asks you to."
        ),
        None => prompt.to_string(),
    }
}

#[derive(Clone)]
struct SpawnParallelWorker {
    definition: AgentDefinition,
    prompt: String,
    task: ParallelAgentTask,
    task_id: String,
    lineage: ParallelAgentLineage,
    worktree_path: Option<PathBuf>,
    workspace_descriptor: Option<WorkspaceDescriptor>,
    dispatch_mode: WorkerDispatchMode,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ParallelAgentLineage {
    pub(super) parent_session: String,
    pub(super) root_session: String,
    pub(super) child_task_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ParallelAgentResult {
    pub(super) task_id: String,
    pub(super) agent_id: String,
    pub(super) lineage: ParallelAgentLineage,
    pub(super) success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) ownership: Option<String>,
    pub(super) elapsed_ms: u64,
    pub(super) iterations: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) stale_parent_reads: Vec<String>,
    /// Absolute path to the worker's isolated `git worktree` checkout, when
    /// it ran with `isolation = "worktree"`. `None` for non-isolated workers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) worktree_path: Option<String>,
    /// Files (relative to the worktree root) the worker changed, collected
    /// from `git status` after the run. Empty for non-isolated workers or a
    /// clean worktree.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) changed_files: Vec<String>,
    /// Whether the worker's worktree had uncommitted changes after the run.
    /// A dirty worktree must not be auto-removed (surfaced to the UI so the
    /// user can choose). `None` for non-isolated workers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) dirty_status: Option<bool>,
}

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
    crate::openhuman::agent_orchestration::subagent_events::publish_subagent_spawned(
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
            crate::openhuman::agent_orchestration::subagent_events::publish_subagent_completed(
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
            crate::openhuman::agent_orchestration::subagent_events::publish_subagent_failed(
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
    let overlaps = crate::openhuman::agent_orchestration::worktree::detect_overlaps(&per_worker);
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
) -> tinyagents::Result<Vec<ParallelAgentResult>> {
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
            use crate::openhuman::agent_orchestration::worktree;
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
    Pin<Box<dyn Future<Output = tinyagents::Result<NodeResult<SpawnParallelUpdate>>> + Send>>;

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
            crate::openhuman::tinyagents::observability::GraphTracingSink::new(label),
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
