mod command_output;
mod current_time;
mod detect_tools;
mod insert_sql_record;
mod install_tool;
mod launch_app;
mod lsp;
mod node_exec;
mod npm_exec;
mod proxy_config;
mod pushover;
mod resolve_time;
mod retrieve_tool_output;
mod schedule;
mod shell;
mod tool_stats;
mod update_apply;
mod update_check;
mod workspace_state;

use crate::openhuman::security::SecurityPolicy;
use tinyagents::harness::tool::ToolExecutionContext;

pub use current_time::CurrentTimeTool;
pub use detect_tools::DetectToolsTool;
pub use insert_sql_record::InsertSqlRecordTool;
pub use install_tool::InstallToolTool;
pub use launch_app::LaunchAppTool;
// Reused by the `automate` inner loop (`desktop-automation`) and the always-on
// voice command router (`voice`) to launch an app mid-flow. Gated to the union of
// its consumers so the fully-slim build (both off) stays warning-clean.
#[cfg(any(feature = "desktop-automation", feature = "voice"))]
pub(crate) use launch_app::launch_platform;
pub use lsp::{lsp_capability_enabled, LspTool, LSP_ENABLED_ENV};
pub use node_exec::NodeExecTool;
pub use npm_exec::NpmExecTool;
pub use proxy_config::ProxyConfigTool;
pub use pushover::PushoverTool;
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
