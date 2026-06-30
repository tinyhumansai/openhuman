//! RPC handler functions for all `openhuman.workflows_*` controllers.
//!
//! Each `handle_*` function deserialises its params, calls into the domain
//! ops layer, and serialises the result back as JSON. Business logic lives in
//! `ops.rs` and skill execution lives in `skill_runtime`; this layer is
//! intentionally thin.

use std::path::Path;

use serde_json::{Map, Value};

use crate::core::all::ControllerFuture;
use crate::openhuman::skill_runtime::spawn_workflow_run_background;
use crate::openhuman::workflows::ops::{
    create_workflow, discover_automations, discover_workflows, install_workflow_from_url,
    is_workspace_trusted, read_workflow_resource, uninstall_workflow, CreateWorkflowParams,
    UninstallWorkflowParams,
};
use crate::openhuman::workflows::{registry, run_log};
use crate::rpc::RpcOutcome;

use super::helpers::{deserialize_params, resolve_config, resolve_workspace_dir, to_json};
use super::wire_types::{
    WorkflowInputDescription, WorkflowSummary, WorkflowsCancelParams, WorkflowsCreateParams,
    WorkflowsCreateResult, WorkflowsDescribeParams, WorkflowsDescribeResult,
    WorkflowsInstallFromUrlParamsWire, WorkflowsInstallFromUrlResult, WorkflowsListParams,
    WorkflowsListResult, WorkflowsReadResourceParams, WorkflowsReadResourceResult,
    WorkflowsReadRunLogParams, WorkflowsRecentRunsParams, WorkflowsRecentRunsResult,
    WorkflowsRunParams, WorkflowsUninstallResult,
};

pub(super) fn handle_workflows_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = deserialize_params::<WorkflowsListParams>(params)?;
        let include_skills = params.include_skills;
        tracing::debug!(include_skills, "[workflows][rpc] list automations");
        let workspace = resolve_workspace_dir().await;
        let trusted = is_workspace_trusted(&workspace);
        let home = dirs::home_dir();
        // Default: automations-only (`workflows/` roots) so capability skills
        // don't masquerade as task templates in the Automations UI. The Skills
        // Explorer passes `include_skills=true` to also surface `skills/`-root
        // installs (registry installs land there) in its Installed tab. Either
        // way the agent harness loads both via `discover_workflows` /
        // `load_workflow_metadata`.
        let listed = if include_skills {
            discover_workflows(home.as_deref(), Some(workspace.as_path()), trusted)
        } else {
            discover_automations(home.as_deref(), Some(workspace.as_path()), trusted)
        };
        tracing::debug!(
            count = listed.len(),
            include_skills,
            workspace = %workspace.display(),
            trusted,
            "[workflows][rpc] list result"
        );
        let summaries = listed.into_iter().map(WorkflowSummary::from).collect();
        to_json(RpcOutcome::new(
            WorkflowsListResult {
                workflows: summaries,
            },
            Vec::new(),
        ))
    })
}

/// `openhuman.workflows_describe` — return a single skill's display metadata
/// and its declared `[[inputs]]` so the Skills Runner panel can render
/// the right form controls. `skills_list` deliberately stays the cheap
/// enumeration without input declarations (its `Workflow` source struct
/// predates `[[inputs]]`); on the user picking one we fetch the full
/// `WorkflowDefinition` (which carries inputs) and project the small,
/// FE-shaped subset they need.
pub(super) fn handle_workflows_describe(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WorkflowsDescribeParams>(params)?;
        let workspace = resolve_workspace_dir().await;
        let skill = registry::get_workflow(&workspace, &payload.workflow_id).ok_or_else(|| {
            format!(
                "workflows_describe: unknown skill '{}'",
                payload.workflow_id
            )
        })?;
        let inputs = skill
            .inputs
            .iter()
            .map(|i| WorkflowInputDescription {
                name: i.name.clone(),
                description: i.description.clone(),
                required: i.required,
                kind: i.kind.clone().unwrap_or_else(|| "string".to_string()),
            })
            .collect();
        let display_name = skill
            .definition
            .display_name
            .clone()
            .unwrap_or_else(|| skill.definition.id.clone());
        to_json(RpcOutcome::new(
            WorkflowsDescribeResult {
                id: skill.definition.id.clone(),
                display_name,
                when_to_use: skill.definition.when_to_use.clone(),
                inputs,
            },
            Vec::new(),
        ))
    })
}

/// `openhuman.workflows_read_run_log` — return a slice of a skill run's
/// log file, identified by `run_id` (NOT a path — no traversal surface).
/// FE Skills Runner panel uses this to render the streaming log inline
/// when the user clicks a Recent Runs row, and tails it every 2s while
/// `complete` is false.
pub(super) fn handle_workflows_read_run_log(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WorkflowsReadRunLogParams>(params)?;
        let workspace = resolve_workspace_dir().await;
        let path = run_log::find_run_log_path(&workspace, &payload.run_id).ok_or_else(|| {
            format!(
                "workflows_read_run_log: unknown run_id '{}'",
                payload.run_id
            )
        })?;
        let offset = payload.offset.unwrap_or(0);
        // 64 KiB default per-call slice, hard cap at 256 KiB to keep the
        // RPC response sane; the FE re-issues with the returned offset
        // to page through larger logs.
        let max_bytes = payload.max_bytes.unwrap_or(64 * 1024).min(256 * 1024) as usize;
        match run_log::read_run_log_slice(&path, offset, max_bytes) {
            Ok(slice) => to_json(RpcOutcome::new(slice, Vec::new())),
            Err(e) => Err(format!("workflows_read_run_log: read failed: {e}")),
        }
    })
}

/// `openhuman.workflows_recent_runs` — list runs from `<workspace>/skills/.runs/`
/// (most-recent first), optionally filtered to one skill, capped by `limit`.
/// Powers the Skills Runner panel's "Recent runs" section + future live-log
/// tail. Delegates the actual scan + parse to `run_log::scan_runs`.
pub(super) fn handle_workflows_recent_runs(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WorkflowsRecentRunsParams>(params)?;
        let limit = payload.limit.unwrap_or(20).min(100) as usize;
        let workspace = resolve_workspace_dir().await;
        let runs = run_log::scan_runs(&workspace, payload.workflow_id.as_deref(), limit);
        tracing::debug!(
            count = runs.len(),
            filter = ?payload.workflow_id,
            limit,
            "[skills][rpc] recent_runs"
        );
        to_json(RpcOutcome::new(
            WorkflowsRecentRunsResult { runs },
            Vec::new(),
        ))
    })
}

pub(super) fn handle_workflows_run(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WorkflowsRunParams>(params)?;
        let started = match spawn_workflow_run_background(payload.workflow_id, payload.inputs).await
        {
            Ok(s) => s,
            Err(e) => return Err(e),
        };
        to_json(RpcOutcome::new(
            serde_json::json!({
                "run_id": started.run_id,
                "status": "started",
                "workflow_id": started.workflow_id,
                "log": started.log_path.display().to_string(),
            }),
            Vec::new(),
        ))
    })
}

/// `openhuman.workflows_cancel` — request cancellation of an in-flight run.
/// Fires the run's cancellation token; the run stops at its next await and
/// writes a `CANCELLED` footer. Returns `cancelled: false` when the run id is
/// unknown (already finished or never existed).
pub(super) fn handle_workflows_cancel(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WorkflowsCancelParams>(params)?;
        let cancelled = run_log::cancel_run(&payload.run_id);
        tracing::info!(run_id = %payload.run_id, cancelled, "[workflows][rpc] cancel");
        to_json(RpcOutcome::new(
            serde_json::json!({ "run_id": payload.run_id, "cancelled": cancelled }),
            Vec::new(),
        ))
    })
}

pub(super) fn handle_workflows_read_resource(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WorkflowsReadResourceParams>(params)?;
        tracing::debug!(
            workflow_id = %payload.workflow_id,
            relative_path = %payload.relative_path,
            "[skills][rpc] read_resource"
        );
        let workspace = resolve_workspace_dir().await;
        let relative = Path::new(&payload.relative_path);
        match read_workflow_resource(workspace.as_path(), &payload.workflow_id, relative) {
            Ok(content) => {
                let bytes = content.len();
                to_json(RpcOutcome::new(
                    WorkflowsReadResourceResult {
                        workflow_id: payload.workflow_id,
                        relative_path: payload.relative_path,
                        content,
                        bytes,
                    },
                    Vec::new(),
                ))
            }
            Err(err) => {
                tracing::debug!(
                    error = %err,
                    "[skills][rpc] read_resource: rejected"
                );
                Err(err)
            }
        }
    })
}

pub(super) fn handle_workflows_create(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WorkflowsCreateParams>(params)?;
        tracing::debug!(
            name = %payload.name,
            scope = ?payload.scope,
            "[skills][rpc] create"
        );
        let workspace = resolve_workspace_dir().await;
        match create_workflow(workspace.as_path(), payload.into()) {
            Ok(skill) => {
                tracing::debug!(
                    skill = %skill.name,
                    location = ?skill.location,
                    "[skills][rpc] create: ok"
                );
                to_json(RpcOutcome::new(
                    WorkflowsCreateResult {
                        workflow: WorkflowSummary::from(skill),
                    },
                    Vec::new(),
                ))
            }
            Err(err) => {
                tracing::debug!(error = %err, "[skills][rpc] create: rejected");
                Err(err)
            }
        }
    })
}

/// `openhuman.workflows_update` — edit an existing workflow. Same payload as
/// create, but overwrites the workflow at the resolved slug (frontmatter +
/// workflow.toml rewritten; the hand-authored body is preserved).
pub(super) fn handle_workflows_update(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WorkflowsCreateParams>(params)?;
        tracing::debug!(
            name = %payload.name,
            scope = ?payload.scope,
            "[workflows][rpc] update"
        );
        let workspace = resolve_workspace_dir().await;
        let mut create_params: CreateWorkflowParams = payload.into();
        create_params.overwrite = true;
        match create_workflow(workspace.as_path(), create_params) {
            Ok(skill) => to_json(RpcOutcome::new(
                WorkflowsCreateResult {
                    workflow: WorkflowSummary::from(skill),
                },
                Vec::new(),
            )),
            Err(err) => {
                tracing::debug!(error = %err, "[workflows][rpc] update: rejected");
                Err(err)
            }
        }
    })
}

pub(super) fn handle_workflows_install_from_url(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let wire = deserialize_params::<WorkflowsInstallFromUrlParamsWire>(params)?;
        tracing::debug!(
            url = %wire.url,
            timeout_secs = ?wire.timeout_secs,
            "[skills][rpc] install_from_url"
        );
        let config = resolve_config().await;
        let workspace = config.workspace_dir.clone();
        let payload = wire.into();
        match install_workflow_from_url(workspace.as_path(), payload).await {
            Ok(outcome) => {
                tracing::debug!(
                    url = %outcome.url,
                    new_count = outcome.new_skills.len(),
                    "[skills][rpc] install_from_url: ok"
                );
                to_json(RpcOutcome::new(
                    WorkflowsInstallFromUrlResult {
                        url: outcome.url,
                        stdout: outcome.stdout,
                        stderr: outcome.stderr,
                        new_workflows: outcome.new_skills,
                    },
                    Vec::new(),
                ))
            }
            Err(err) => {
                tracing::debug!(error = %err, "[skills][rpc] install_from_url: rejected");
                Err(err)
            }
        }
    })
}

pub(super) fn handle_workflows_uninstall(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<UninstallWorkflowParams>(params)?;
        tracing::debug!(name = %payload.name, "[skills][rpc] uninstall");
        match uninstall_workflow(payload, None) {
            Ok(outcome) => {
                tracing::debug!(
                    name = %outcome.name,
                    removed_path = %outcome.removed_path,
                    "[skills][rpc] uninstall: ok"
                );
                to_json(RpcOutcome::new(
                    WorkflowsUninstallResult {
                        name: outcome.name,
                        removed_path: outcome.removed_path,
                        scope: outcome.scope,
                    },
                    Vec::new(),
                ))
            }
            Err(err) => {
                tracing::debug!(error = %err, "[skills][rpc] uninstall: rejected");
                Err(err)
            }
        }
    })
}
