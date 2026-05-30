//! Agent tools for the `agent_workflows` domain.
//!
//! Two tools are provided:
//!
//! * **`workflow_load`** — read-only; returns the full workflow definition
//!   (name, description, phases with their rules/scripts/tools/context) as a
//!   human-readable text block. The model uses this to inspect what a workflow
//!   does before deciding whether to activate it.
//!
//! * **`workflow_phase`** — execute-class; activates a specific phase of a
//!   workflow. It (a) reads the workflow from disk, (b) runs each phase script
//!   through the security-gated [`ShellTool`] path (same rate-limits,
//!   path-guards, and `ApprovalGate` routing as a raw `shell` call), (c)
//!   builds working-directory context via
//!   [`crate::openhuman::agent_workflows::working_dir_context`], and (d)
//!   returns a combined result containing the rendered phase guidance, the
//!   effective tool scope, the working-dir context block, and each script's
//!   gated output.
//!
//! Because `workflow_phase` runs shell scripts it is marked `external_effect`
//! / `PermissionLevel::Execute` so the harness routes it through the
//! `ApprovalGate` exactly like a raw `shell` call.

use crate::openhuman::agent::host_runtime::RuntimeAdapter;
use crate::openhuman::agent_workflows::{
    effective_tool_scope, phase_guidance, read_workflow, working_dir_context,
};
use crate::openhuman::security::{AuditLogger, SecurityPolicy};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use crate::openhuman::tools::ShellTool;
use async_trait::async_trait;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

/// Maximum number of characters to retain from a single script's output.
/// Long output is truncated and an ellipsis marker appended so the context
/// window is not dominated by one verbose script.
const MAX_SCRIPT_OUTPUT_CHARS: usize = 4_096;

// ─────────────────────────────────────────────────────────────────────────────
// WorkflowLoadTool
// ─────────────────────────────────────────────────────────────────────────────

/// Read-only tool that returns the full definition of a workflow.
///
/// The model calls this when it wants to inspect a workflow's phases, rules,
/// and scripts before deciding to activate one.
pub struct WorkflowLoadTool;

#[async_trait]
impl Tool for WorkflowLoadTool {
    fn name(&self) -> &str {
        "workflow_load"
    }

    fn description(&self) -> &str {
        "Load and inspect a workflow definition by its id (directory slug). \
         Returns the workflow name, description, and for each phase: its \
         description, rules, scripts, tool-scope, and working-dir context \
         providers. Use this before calling workflow_phase to understand what \
         a workflow does."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["id"],
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Workflow id — the directory slug (e.g. \
                                   \"github-issue-crusher\"). Use \
                                   agent_workflows_list to discover available ids."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn external_effect(&self) -> bool {
        false
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let id = match args.get("id").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return Ok(ToolResult::error(
                    "missing required argument: id".to_string(),
                ));
            }
        };

        log::debug!("[workflows][phase] workflow_load invoked id={:?}", id);

        match read_workflow(id) {
            Err(e) => {
                log::warn!(
                    "[workflows][phase] workflow_load failed id={:?} err={}",
                    id,
                    e
                );
                Ok(ToolResult::error(format!(
                    "workflow not found or could not be read: {e}"
                )))
            }
            Ok(workflow) => {
                let mut out = String::with_capacity(2048);
                out.push_str(&format!(
                    "# Workflow: {}\n\nId: {}\nDescription: {}\n",
                    workflow.name, workflow.dir_name, workflow.description
                ));
                if !workflow.when_to_use.is_empty() {
                    out.push_str(&format!("When to use: {}\n", workflow.when_to_use));
                }
                if !workflow.tags.is_empty() {
                    out.push_str(&format!("Tags: {}\n", workflow.tags.join(", ")));
                }
                if !workflow.phases.is_empty() {
                    out.push_str("\n## Phases\n");
                    let mut phase_names: Vec<&String> = workflow.phases.keys().collect();
                    phase_names.sort();
                    for phase_name in phase_names {
                        let phase = &workflow.phases[phase_name];
                        out.push_str(&format!("\n### {}\n", phase_name));
                        if let Some(desc) = &phase.description {
                            out.push_str(&format!("Description: {}\n", desc));
                        }
                        if !phase.rules.is_empty() {
                            out.push_str("Rules:\n");
                            for rule in &phase.rules {
                                out.push_str(&format!("  - {}\n", rule));
                            }
                        }
                        if !phase.scripts.is_empty() {
                            out.push_str("Scripts:\n");
                            for script in &phase.scripts {
                                out.push_str(&format!("  $ {}\n", script));
                            }
                        }
                        if let Some(tools) = &phase.tools {
                            if !tools.allow.is_empty() {
                                out.push_str(&format!(
                                    "Tool scope (allow): {}\n",
                                    tools.allow.join(", ")
                                ));
                            }
                            if !tools.deny.is_empty() {
                                out.push_str(&format!(
                                    "Tool scope (deny): {}\n",
                                    tools.deny.join(", ")
                                ));
                            }
                        }
                        if !phase.context.is_empty() {
                            out.push_str(&format!(
                                "Context providers: {}\n",
                                phase.context.join(", ")
                            ));
                        }
                    }
                }
                if !workflow.warnings.is_empty() {
                    out.push_str("\n## Warnings\n");
                    for w in &workflow.warnings {
                        out.push_str(&format!("  ! {}\n", w));
                    }
                }

                log::debug!(
                    "[workflows][phase] workflow_load success id={:?} phases={} output_chars={}",
                    id,
                    workflow.phases.len(),
                    out.len()
                );
                Ok(ToolResult::success(out))
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// WorkflowPhaseTool
// ─────────────────────────────────────────────────────────────────────────────

/// Execute-class tool that activates a named phase of an installed workflow.
///
/// On activation the tool:
/// 1. Reads the workflow definition from disk.
/// 2. Runs each `phase.scripts` entry through the security-gated
///    [`ShellTool::run_with_security`] path — same rate-limits, path-guards,
///    and `ApprovalGate` routing as a raw `shell` call.
/// 3. Builds working-directory context via
///    [`crate::openhuman::agent_workflows::working_dir_context`].
/// 4. Returns a combined block containing: rendered phase guidance, the
///    effective tool scope, the working-dir context, and each script's
///    gated output (truncated to [`MAX_SCRIPT_OUTPUT_CHARS`]).
///
/// Because this tool runs shell scripts it is marked `external_effect` and
/// `PermissionLevel::Execute` so the harness routes it through the
/// `ApprovalGate` before `execute()` is called.
pub struct WorkflowPhaseTool {
    workspace_dir: PathBuf,
    security: Arc<SecurityPolicy>,
    runtime: Arc<dyn RuntimeAdapter>,
    audit: Arc<AuditLogger>,
}

impl WorkflowPhaseTool {
    /// Create a new `WorkflowPhaseTool`.
    ///
    /// The `security`, `runtime`, and `audit` arcs should come from the same
    /// session-scoped instances used to construct the session's `ShellTool`,
    /// ensuring rate-limit counters and audit trails are shared.
    pub fn new(
        workspace_dir: PathBuf,
        security: Arc<SecurityPolicy>,
        runtime: Arc<dyn RuntimeAdapter>,
        audit: Arc<AuditLogger>,
    ) -> Self {
        Self {
            workspace_dir,
            security,
            runtime,
            audit,
        }
    }
}

#[async_trait]
impl Tool for WorkflowPhaseTool {
    fn name(&self) -> &str {
        "workflow_phase"
    }

    fn description(&self) -> &str {
        "Activate a named phase of an installed workflow. The tool runs the \
         phase's gated scripts (same security policy as the shell tool), \
         surfaces working-directory context (e.g. git status), renders the \
         phase's guidance rules, and returns the effective tool scope so you \
         know which tools are allowed for the rest of the task. Call \
         workflow_load first to inspect what a workflow does."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["id", "phase"],
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Workflow id — the directory slug (e.g. \
                                   \"github-issue-crusher\")."
                },
                "phase": {
                    "type": "string",
                    "description": "Phase name to activate, e.g. \
                                   \"on_pick_up_task\", \"on_close_task\", \
                                   or \"on_enter_directory\"."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // Phase scripts run through the shell — treat as Execute so the
        // harness applies the appropriate gate.
        PermissionLevel::Execute
    }

    fn external_effect(&self) -> bool {
        // Phase scripts may have external effects; always route through
        // the ApprovalGate.
        true
    }

    fn external_effect_with_args(&self, _args: &serde_json::Value) -> bool {
        true
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let id = match args.get("id").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return Ok(ToolResult::error(
                    "missing required argument: id".to_string(),
                ));
            }
        };
        let phase_name = match args.get("phase").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return Ok(ToolResult::error(
                    "missing required argument: phase".to_string(),
                ));
            }
        };

        log::info!(
            "[workflows][phase] workflow_phase invoked id={:?} phase={:?}",
            id,
            phase_name
        );

        // (a) Read the workflow.
        let workflow = match read_workflow(id) {
            Ok(w) => w,
            Err(e) => {
                log::warn!(
                    "[workflows][phase] workflow_phase: workflow not found id={:?} err={}",
                    id,
                    e
                );
                return Ok(ToolResult::error(format!(
                    "workflow not found or could not be read: {e}"
                )));
            }
        };

        // Locate the requested phase.
        let phase = match workflow.phases.get(phase_name) {
            Some(p) => p,
            None => {
                log::warn!(
                    "[workflows][phase] workflow_phase: phase not found id={:?} phase={:?} \
                     available={:?}",
                    id,
                    phase_name,
                    workflow.phase_names()
                );
                return Ok(ToolResult::error(format!(
                    "phase {:?} not found in workflow {:?}; available phases: {}",
                    phase_name,
                    id,
                    workflow.phase_names().join(", ")
                )));
            }
        };

        let mut out = String::with_capacity(4096);

        // (b) Run phase scripts through the gated shell path.
        if !phase.scripts.is_empty() {
            log::debug!(
                "[workflows][phase] running {} script(s) for id={:?} phase={:?}",
                phase.scripts.len(),
                id,
                phase_name
            );
            let shell = ShellTool::new(
                Arc::clone(&self.security),
                Arc::clone(&self.runtime),
                Arc::clone(&self.audit),
            );
            out.push_str("## Script Results\n\n");
            for (i, script) in phase.scripts.iter().enumerate() {
                log::debug!(
                    "[workflows][phase] executing script #{} id={:?} phase={:?} cmd={:?}",
                    i + 1,
                    id,
                    phase_name,
                    script
                );
                let (allowed, result) = shell.run_with_security(script).await;
                let raw_output = result.output();
                let truncated = raw_output.len() > MAX_SCRIPT_OUTPUT_CHARS;
                let display_output = if truncated {
                    format!(
                        "{}…[output truncated at {} chars]",
                        &raw_output[..MAX_SCRIPT_OUTPUT_CHARS],
                        raw_output.len()
                    )
                } else {
                    raw_output.to_string()
                };

                log::debug!(
                    "[workflows][phase] script #{} completed id={:?} phase={:?} \
                     allowed={} is_error={} output_chars={}",
                    i + 1,
                    id,
                    phase_name,
                    allowed,
                    result.is_error,
                    raw_output.len()
                );

                out.push_str(&format!("### Script: `{}`\n", script));
                if !allowed {
                    out.push_str("Status: BLOCKED by security policy\n");
                } else if result.is_error {
                    out.push_str("Status: ERROR\n");
                } else {
                    out.push_str("Status: OK\n");
                }
                if !display_output.trim().is_empty() {
                    out.push_str("Output:\n```\n");
                    out.push_str(display_output.trim_end());
                    out.push_str("\n```\n");
                }
                out.push('\n');
            }
        }

        // (c) Build working-directory context.
        if !phase.context.is_empty() {
            log::debug!(
                "[workflows][phase] building working-dir context id={:?} phase={:?} providers={:?}",
                id,
                phase_name,
                phase.context
            );
            let ctx = working_dir_context(&self.workspace_dir, &phase.context);
            if !ctx.trim().is_empty() {
                out.push_str("## Working Directory Context\n\n");
                out.push_str(ctx.trim_end());
                out.push_str("\n\n");
            }
        }

        // (d) Phase guidance (rules).
        if let Some(guidance) = phase_guidance(&workflow, phase_name) {
            if !guidance.trim().is_empty() {
                out.push_str("## Phase Guidance\n\n");
                out.push_str(guidance.trim_end());
                out.push_str("\n\n");
            }
        }

        // Effective tool scope.
        if let Some(scope) = effective_tool_scope(&workflow, phase_name) {
            let mut scope_lines = Vec::new();
            if !scope.allow.is_empty() {
                scope_lines.push(format!("Allow: {}", scope.allow.join(", ")));
            }
            if !scope.deny.is_empty() {
                scope_lines.push(format!("Deny: {}", scope.deny.join(", ")));
            }
            if !scope_lines.is_empty() {
                out.push_str("## Effective Tool Scope\n\n");
                for line in scope_lines {
                    out.push_str(&format!("{}\n", line));
                }
                out.push('\n');
            }
        }

        if out.trim().is_empty() {
            out.push_str(&format!(
                "Phase {:?} of workflow {:?} activated (no scripts, context, or guidance configured).\n",
                phase_name, id
            ));
        }

        log::info!(
            "[workflows][phase] workflow_phase complete id={:?} phase={:?} output_chars={}",
            id,
            phase_name,
            out.len()
        );

        Ok(ToolResult::success(out))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Unit tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "workflow_tools_tests.rs"]
mod tests;
