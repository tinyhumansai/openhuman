use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use tinyagents_graph::export::GraphTopology;
use tinyagents_graph::parallel::{
    map_reduce, parse_relative_claim_paths, plan_shared_workspace_dispatch, ClaimConflict,
    ClaimPathError, DispatchMode, FailurePolicy, ParallelOptions, WorkspaceClaim,
};
use tinyagents_graph::{
    ClosureStateReducer, CompiledGraph, GraphBuilder, NodeContext, NodeResult,
};
use tinyagents_harness::retry::RetryPolicy;
use tinyagents_harness::workspace::{WorkspaceDescriptor, WorkspaceIsolation};
use tinyagents_harness::{CancellationToken, TinyAgentsError};

use crate::openhuman::agent::file_state;
use crate::openhuman::agent::harness::definition::{
    AgentDefinition, AgentDefinitionRegistry, SandboxMode, ToolScope,
};
use crate::openhuman::agent::harness::fork_context::{current_parent, ParentExecutionContext};
use crate::openhuman::agent::harness::subagent_runner::{run_subagent, SubagentRunOptions};
use crate::openhuman::agent::orchestration::worktree::{self, BaseRef};
use crate::openhuman::agent::progress::AgentProgress;
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

impl PreparedParallelTask {
    /// How this worker will be dispatched relative to its siblings.
    ///
    /// Exposed so the write-safety decision can be asserted directly: it is the
    /// one property of a preflight whose regression corrupts a shared checkout
    /// silently rather than failing a run.
    pub(crate) fn dispatch_mode(&self) -> WorkerDispatchMode {
        self.dispatch_mode
    }
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
pub(crate) enum WorkerDispatchMode {
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
                        || (crate::openhuman::inference::tokenjuice::is_recovery_tool(name)
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

/// Parse OpenHuman's `files: a.rs, b.rs` ownership syntax into claimed paths.
///
/// The `files:` prefix is this tool's parameter shape, so it is stripped here;
/// validating what follows is generic and belongs to
/// [`parse_relative_claim_paths`]. Its typed rejection is rendered into the
/// sentence the model reads at this boundary, which is why the crate returns
/// data rather than a message.
fn ownership_file_paths(ownership: Option<&str>) -> Result<Vec<PathBuf>, String> {
    let Some(ownership) = ownership.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(Vec::new());
    };
    let Some(rest) = ownership.strip_prefix("files:") else {
        return Ok(Vec::new());
    };
    parse_relative_claim_paths(rest).map_err(|err| {
        let raw = match &err {
            ClaimPathError::Absolute { raw } | ClaimPathError::Escaping { raw } => raw,
        };
        format!("ownership path '{raw}' must be a relative file path under the workspace")
    })
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
                    SandboxMode::Sandboxed => tinyagents_harness::tool::SandboxMode::Required,
                    SandboxMode::None | SandboxMode::ReadOnly => {
                        tinyagents_harness::tool::SandboxMode::Inherit
                    }
                };
                let isolation = worktree::OpenHumanWorktreeIsolation::new(repo_root)
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

/// One task that cleared every OpenHuman policy gate and is awaiting the
/// shared-workspace arbitration verdict.
struct AdmittedParallelTask {
    definition: AgentDefinition,
    prompt: String,
    task: ParallelAgentTask,
    task_id: String,
    /// What this worker needs from the shared workspace, in the crate's terms.
    claim: WorkspaceClaim,
}

pub(super) fn prepare_spawn_parallel_tasks_from_defs(
    tasks: Vec<ParallelAgentTask>,
    definitions: &HashMap<String, AgentDefinition>,
    parent: &ParentExecutionContext,
) -> Vec<SpawnParallelTaskPreflight> {
    // Pass 1 — OpenHuman policy. Identity, the parent's subagent allowlist, the
    // integrations toolkit requirement, and whether a worker can write the
    // shared workspace at all are all product decisions, so they are settled
    // here and rejected in their own vocabulary. What survives carries a
    // `WorkspaceClaim` describing only what the arbiter needs to know.
    enum Admission {
        Admitted(Box<AdmittedParallelTask>),
        Rejected(ParallelTaskRejection),
    }

    let admissions: Vec<Admission> = tasks
        .into_iter()
        .map(|task| {
            let agent_id = task.agent_id.trim().to_string();
            let prompt = task.prompt.trim().to_string();
            let task_id = format!("sub-{}", uuid::Uuid::new_v4());

            if agent_id.is_empty() || prompt.is_empty() {
                return Admission::Rejected(ParallelTaskRejection {
                    task_id,
                    agent_id,
                    error: "agent_id and prompt are required".to_string(),
                    ownership: task.ownership,
                    kind: ParallelTaskRejectionKind::MissingAgentOrPrompt,
                });
            }

            let Some(definition) = definitions.get(&agent_id).cloned() else {
                return Admission::Rejected(ParallelTaskRejection {
                    task_id,
                    agent_id: agent_id.clone(),
                    error: format!("unknown agent_id '{agent_id}'"),
                    ownership: task.ownership,
                    kind: ParallelTaskRejectionKind::UnknownAgent,
                });
            };

            if !parent.allowed_subagent_ids.contains(&definition.id) {
                return Admission::Rejected(ParallelTaskRejection {
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
                return Admission::Rejected(ParallelTaskRejection {
                    task_id,
                    agent_id,
                    error: "integrations_agent requires toolkit".to_string(),
                    ownership: task.ownership,
                    kind: ParallelTaskRejectionKind::MissingToolkit,
                });
            }

            let claim = match shared_workspace_write_claim(&task, &definition, parent) {
                // Needs the shared workspace and declares what it owns.
                Ok(Some(paths)) => WorkspaceClaim::writing(task_id.clone(), paths),
                // Cannot collide. Both arms plan as parallel, but they say so
                // for different reasons and the claim should carry the real one:
                // an isolated worker has its own root, a shared one simply never
                // writes.
                Ok(None) => {
                    if matches!(
                        worktree_request_for_task(&task),
                        ParallelWorktreeRequest::Isolated { .. }
                    ) {
                        WorkspaceClaim::isolated(task_id.clone())
                    } else {
                        WorkspaceClaim::read_only(task_id.clone())
                    }
                }
                Err(error) => {
                    return Admission::Rejected(ParallelTaskRejection {
                        task_id,
                        agent_id: definition.id.clone(),
                        error,
                        ownership: task.ownership,
                        kind: ParallelTaskRejectionKind::RequiresIsolation,
                    });
                }
            };

            Admission::Admitted(Box::new(AdmittedParallelTask {
                definition,
                prompt,
                task,
                task_id,
                claim,
            }))
        })
        .collect();

    // Pass 2 — arbitration. One planner call over every admitted claim, in input
    // order, so the verdict is a pure function of the request rather than of
    // which worker happened to be considered first. `shared_workspace_write_claim`
    // has already ruled out the unbounded-write case, so the only conflict the
    // planner can report here is an overlap.
    let claims: Vec<WorkspaceClaim> = admissions
        .iter()
        .filter_map(|admission| match admission {
            Admission::Admitted(admitted) => Some(admitted.claim.clone()),
            Admission::Rejected(_) => None,
        })
        .collect();
    let plan = plan_shared_workspace_dispatch(&claims);
    let conflicts: HashMap<usize, &ClaimConflict> = plan
        .conflicts
        .iter()
        .map(|(index, conflict)| (*index, conflict))
        .collect();

    let mut admitted_index = 0usize;
    admissions
        .into_iter()
        .map(|admission| {
            let admitted = match admission {
                Admission::Rejected(rejection) => {
                    return SpawnParallelTaskPreflight::Rejected(rejection);
                }
                Admission::Admitted(admitted) => admitted,
            };
            let index = admitted_index;
            admitted_index += 1;

            let AdmittedParallelTask {
                definition,
                prompt,
                task,
                task_id,
                claim: _,
            } = *admitted;

            if let Some(conflict) = conflicts.get(&index) {
                return SpawnParallelTaskPreflight::Rejected(ParallelTaskRejection {
                    task_id,
                    agent_id: definition.id.clone(),
                    error: shared_workspace_conflict_message(&definition.id, conflict),
                    ownership: task.ownership,
                    kind: ParallelTaskRejectionKind::RequiresIsolation,
                });
            }

            let dispatch_mode = match plan.modes.get(index).copied().flatten() {
                Some(DispatchMode::Serial) => WorkerDispatchMode::SerialSharedWorkspaceWrite,
                // A claim the planner neither serialized nor rejected cannot
                // collide, so it is safe to fan out.
                Some(DispatchMode::Parallel) | None => WorkerDispatchMode::Parallel,
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

/// Render a claim conflict as the sentence the calling model reads.
///
/// The crate reports conflicts as data precisely so this phrasing — the
/// `isolation="worktree"` remedy, the `files:` vocabulary — stays a product
/// decision rather than becoming API.
fn shared_workspace_conflict_message(agent_id: &str, conflict: &ClaimConflict) -> String {
    match conflict {
        ClaimConflict::Overlap {
            other_worker_id,
            path,
            ..
        } => format!(
            "agent '{agent_id}' requested shared-workspace write access to '{}' but it overlaps with serial worker {other_worker_id}; set isolation=\"worktree\" or use disjoint files: ownership",
            path.display()
        ),
        ClaimConflict::UnboundedWrite { .. } => format!(
            "agent '{agent_id}' can write the shared workspace without declaring which files it owns; set isolation=\"worktree\" or provide disjoint files: ownership"
        ),
    }
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
