//! OpenHuman policy admission for a `spawn_parallel_agents` task batch:
//! identity, the parent's subagent allowlist, and whether a worker can write
//! the shared workspace at all — plus the crate-side arbitration over the resulting workspace claims.
//!
//! **Write safety.** Whether a worker *needs* a claim on the shared workspace is
//! an OpenHuman decision — it reads sandbox mode, tool permissions and the
//! isolation request. Whether the claims of a whole batch can be granted
//! together is not, and goes through
//! [`plan_shared_workspace_dispatch`](tinyagents_graph::parallel::plan_shared_workspace_dispatch).
//! The rejection sentences stay here, which is why the crate reports conflicts
//! as data.

use std::collections::HashMap;
use std::path::PathBuf;

use std::cell::RefCell;

use tinyagents_graph::parallel::{parse_relative_claim_paths, ClaimConflict, ClaimPathError};
use tinyagents_orchestration::parallel::WorkerDispatchMode;
use tinyagents_orchestration::parallel::{
    AdmissionPolicy, AdmissionVerdict, Plan, Preflight, Rejection, RejectionKind, Task,
};

use crate::agent::harness::fork_context::ParentExecutionContext;
use crate::agent::harness::host_definition::{AgentDefinition, AgentDefinitionRegistry};
use tinyagents_definition::{SandboxMode, ToolScope};
use tinyagents_harness::workspace::GitWorktreeBaseRef;
use tinytools::PermissionLevel;

use super::types::ParallelAgentTask;

/// Prepared worker ready for the live dispatch/worker phases.
pub(crate) struct PreparedParallelTask {
    pub(crate) definition: AgentDefinition,
    pub(crate) prompt: String,
    pub(crate) task: ParallelAgentTask,
    pub(crate) task_id: String,
    pub(crate) dispatch_mode: WorkerDispatchMode,
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
pub(crate) enum ParallelWorktreeRequest {
    SharedWorkspace,
    Isolated { base_ref: GitWorktreeBaseRef },
}

pub(crate) fn worktree_request_for_task(task: &ParallelAgentTask) -> ParallelWorktreeRequest {
    let isolated = task
        .isolation
        .as_deref()
        .map(str::trim)
        .map(|s| s.eq_ignore_ascii_case("worktree"))
        .unwrap_or(false);
    if isolated {
        ParallelWorktreeRequest::Isolated {
            base_ref: GitWorktreeBaseRef::parse(task.base_ref.as_deref()),
        }
    } else {
        ParallelWorktreeRequest::SharedWorkspace
    }
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
            if crate::tools::rules::glob_list_matches(&definition.disallowed_tools, name) {
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
                        || (crate::inference::tokenjuice::is_recovery_tool(name)
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

pub(crate) fn snapshot_agent_definitions(
    registry: &AgentDefinitionRegistry,
) -> HashMap<String, AgentDefinition> {
    registry
        .list()
        .into_iter()
        .map(|definition| (definition.id.clone(), definition.clone()))
        .collect()
}

/// Host payload carried through the module's [`Plan`]: the decoded request and,
/// once the agent id resolved, its definition.
struct AdmitPayload {
    task: ParallelAgentTask,
    /// Trimmed requested id, or the resolved definition id.
    agent_id: String,
    definition: Option<AgentDefinition>,
}

/// OpenHuman policy over the module's admission seam: identity, the parent's
/// subagent allowlist, and whether a worker can write the shared workspace at
/// all. The module owns arbitration and prompt framing.
struct SpawnParallelPolicy<'a> {
    parent: &'a ParentExecutionContext,
    /// Rejection kind per input index (the module carries only the sentence).
    kinds: RefCell<HashMap<usize, ParallelTaskRejectionKind>>,
}

impl SpawnParallelPolicy<'_> {
    fn reject(
        &self,
        index: usize,
        kind: ParallelTaskRejectionKind,
        message: String,
    ) -> AdmissionVerdict {
        self.kinds.borrow_mut().insert(index, kind);
        AdmissionVerdict::Reject(message)
    }
}

impl AdmissionPolicy<AdmitPayload> for SpawnParallelPolicy<'_> {
    fn admit(&self, index: usize, task: &Task<AdmitPayload>) -> AdmissionVerdict {
        let payload = &task.payload;
        if payload.agent_id.is_empty() || task.prompt.is_empty() {
            return self.reject(
                index,
                ParallelTaskRejectionKind::MissingAgentOrPrompt,
                "agent_id and prompt are required".to_string(),
            );
        }
        let Some(definition) = payload.definition.as_ref() else {
            return self.reject(
                index,
                ParallelTaskRejectionKind::UnknownAgent,
                format!("unknown agent_id '{}'", payload.agent_id),
            );
        };
        if !self.parent.allowed_subagent_ids.contains(&definition.id) {
            return self.reject(
                index,
                ParallelTaskRejectionKind::OutsideAllowlist,
                format!(
                    "agent '{}' is not in parent agent '{}' subagents.allowlist",
                    definition.id, self.parent.agent_definition_id
                ),
            );
        }
        match shared_workspace_write_claim(&payload.task, definition, self.parent) {
            Ok(Some(paths)) => AdmissionVerdict::Writes(paths),
            Ok(None) => {
                if matches!(
                    worktree_request_for_task(&payload.task),
                    ParallelWorktreeRequest::Isolated { .. }
                ) {
                    AdmissionVerdict::Isolated
                } else {
                    AdmissionVerdict::ReadOnly
                }
            }
            Err(error) => self.reject(index, ParallelTaskRejectionKind::RequiresIsolation, error),
        }
    }
}

pub(crate) fn prepare_spawn_parallel_tasks_from_defs(
    tasks: Vec<ParallelAgentTask>,
    definitions: &HashMap<String, AgentDefinition>,
    parent: &ParentExecutionContext,
) -> Vec<SpawnParallelTaskPreflight> {
    let plan = Plan::new(
        tasks
            .into_iter()
            .map(|task| {
                let requested = task.agent_id.trim().to_string();
                let definition = definitions.get(&requested).cloned();
                Task {
                    id: format!("sub-{}", uuid::Uuid::new_v4()),
                    prompt: task.prompt.trim().to_string(),
                    ownership: task.ownership.clone(),
                    payload: AdmitPayload {
                        agent_id: requested,
                        definition,
                        task,
                    },
                }
            })
            .collect(),
    );
    let policy = SpawnParallelPolicy {
        parent,
        kinds: RefCell::new(HashMap::new()),
    };
    let staged = plan.stage(&policy);
    let kinds = policy.kinds.into_inner();
    staged
        .into_iter()
        .enumerate()
        .map(|(index, item)| match item {
            Preflight::Prepared(worker) => {
                let AdmitPayload {
                    task, definition, ..
                } = worker.payload;
                SpawnParallelTaskPreflight::Prepared(PreparedParallelTask {
                    definition: definition.expect("admitted task has a definition"),
                    prompt: worker.prompt,
                    task,
                    task_id: worker.id,
                    dispatch_mode: worker.dispatch_mode,
                })
            }
            Preflight::Rejected(rejection) => {
                let Rejection { task, kind } = rejection;
                let agent_id = task
                    .payload
                    .definition
                    .as_ref()
                    .map(|d| d.id.clone())
                    .unwrap_or_else(|| task.payload.agent_id.clone());
                let (error, kind) = match kind {
                    RejectionKind::Policy(message) => (
                        message,
                        kinds
                            .get(&index)
                            .copied()
                            .unwrap_or(ParallelTaskRejectionKind::RequiresIsolation),
                    ),
                    RejectionKind::Conflict(conflict) => (
                        shared_workspace_conflict_message(&agent_id, &conflict),
                        ParallelTaskRejectionKind::RequiresIsolation,
                    ),
                };
                SpawnParallelTaskPreflight::Rejected(ParallelTaskRejection {
                    task_id: task.id,
                    agent_id,
                    error,
                    ownership: task.ownership,
                    kind,
                })
            }
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
