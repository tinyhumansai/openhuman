mod command_output;
mod current_time;
mod detect_tools;
mod insert_sql_record;
mod install_tool;
mod lsp;
mod node_exec;
mod npm_exec;
mod proxy_config;
mod pushover;
mod python_exec;
mod resolve_time;
mod retrieve_tool_output;
mod schedule;
mod shell;
mod tool_stats;
mod update_apply;
mod update_check;
mod workspace_state;

use crate::openhuman::security::SecurityPolicy;
use std::path::Path;
use tinyagents::harness::tool::ToolExecutionContext;

pub use current_time::CurrentTimeTool;
pub use detect_tools::DetectToolsTool;
pub use insert_sql_record::InsertSqlRecordTool;
pub use install_tool::InstallToolTool;
pub use lsp::{lsp_capability_enabled, LspTool, LSP_ENABLED_ENV};
pub use node_exec::NodeExecTool;
pub use npm_exec::NpmExecTool;
pub use proxy_config::ProxyConfigTool;
pub use pushover::PushoverTool;
pub use python_exec::PythonExecTool;
pub use resolve_time::ResolveTimeTool;
pub use retrieve_tool_output::RetrieveToolOutputTool;
pub use schedule::ScheduleTool;
pub use shell::ShellTool;
pub use tool_stats::ToolStatsTool;
pub use update_apply::UpdateApplyTool;
pub use update_check::UpdateCheckTool;
pub use workspace_state::WorkspaceStateTool;

pub(super) fn security_for_tool_context(
    security: &SecurityPolicy,
    context: Option<&ToolExecutionContext>,
    tool: &str,
) -> SecurityPolicy {
    let mut scoped = security.clone();
    if let Some(workspace) = context.and_then(|ctx| ctx.workspace.as_ref()) {
        tracing::debug!(
            tool,
            workspace_root = %workspace.root.display(),
            policy_id = %workspace.policy_id,
            "[tools:system] using TinyAgents workspace descriptor as action dir"
        );
        scoped.action_dir = workspace.root.clone();
    }
    scoped
}

/// Apply the dedicated-workspace profile boundary to an arbitrary process
/// command before it is spawned. Process tools do not funnel their runtime file
/// writes through `SecurityPolicy::validate_path`, so shell, Node, and npm must
/// all share this defense-in-depth scan.
pub(super) fn check_cross_profile_command(
    security: &SecurityPolicy,
    command: &str,
    cwd: &Path,
    tool: &str,
) -> Result<(), String> {
    let Some(guard) = security.active_profile.as_ref() else {
        return Ok(());
    };
    // Classify cwd itself before scanning command tokens. A process tool may
    // accept a syntactically in-profile directory that is actually a symlink
    // into a sibling; once spawned there, npm lifecycle hooks or a shell can
    // mutate that sibling without mentioning its path in the command.
    let other_id = match crate::openhuman::agent::profiles::classify_cross_profile_target(
        &guard.action_dir,
        &guard.profile_id,
        cwd,
    ) {
        crate::openhuman::agent::profiles::CrossProfileDecision::Block { other_id } => {
            Some(other_id)
        }
        crate::openhuman::agent::profiles::CrossProfileDecision::Allow => {
            crate::openhuman::agent::profiles::scan_command_for_cross_profile(
                command,
                cwd,
                &guard.action_dir,
                &guard.profile_id,
            )
        }
    };
    let Some(other_id) = other_id else {
        return Ok(());
    };

    tracing::warn!(
        tool,
        active_profile = %guard.profile_id,
        other_profile = %other_id,
        "[profiles] cross-profile process command blocked"
    );
    if other_id == crate::openhuman::agent::profiles::PROFILES_ROOT_SENTINEL {
        Err(format!(
            "{} Cross-profile access blocked: profile '{}' may not modify the shared profiles \
             root. Stay within your own profile directory; do not retry this command.",
            crate::openhuman::security::POLICY_BLOCKED_MARKER,
            guard.profile_id,
        ))
    } else {
        Err(format!(
            "{} Cross-profile access blocked: profile '{}' may not touch profile '{}'s workspace. \
             Stay within your own profile directory; do not retry this command.",
            crate::openhuman::security::POLICY_BLOCKED_MARKER,
            guard.profile_id,
            other_id
        ))
    }
}
