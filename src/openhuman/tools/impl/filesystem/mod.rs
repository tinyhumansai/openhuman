mod apply_patch;
mod csv_export;
mod edit_file;
mod file_read;
mod file_write;
mod git_operations;
mod git_operations_config;
mod git_operations_render;
mod glob_search;
mod grep;
mod list_files;
mod read_diff;
mod run_linter;
mod run_tests;
mod update_memory_md;
mod write_sink;

use crate::openhuman::security::policy::{TrustedAccess, TrustedRoot};
use crate::openhuman::security::SecurityPolicy;
use tinytools::ToolRunContext;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

/// The git config overrides a shell-spawned `git` is forced to run under.
///
/// Re-exported from the module that owns the whole policy so the `shell` tool
/// and `git_operations` cannot drift into two different answers about which
/// config keys are dangerous.
pub(crate) use git_operations_config::SHELL_NEUTRALISED_CONFIG;

pub use apply_patch::ApplyPatchTool;
pub use csv_export::CsvExportTool;
pub use edit_file::EditFileTool;
pub use file_read::FileReadTool;
pub use file_write::FileWriteTool;
pub use git_operations::GitOperationsTool;
pub use glob_search::GlobTool;
pub use grep::GrepTool;
pub use list_files::ListFilesTool;
pub use read_diff::ReadDiffTool;
pub use run_linter::RunLinterTool;
pub use run_tests::RunTestsTool;
pub use update_memory_md::UpdateMemoryMdTool;

/// Clone `security` and scope it to the run's workspace descriptor, if any.
///
/// The descriptor's root becomes both the relative-path resolution root
/// (`action_dir`) **and** a `ReadWrite` trusted root. The grant is the
/// load-bearing half: `action_dir` only decides where a relative path lands,
/// while the allow/deny decision reads `workspace_dir` + `trusted_roots`
/// (`SecurityPolicy::is_resolved_path_allowed_for`). Without the grant a
/// descriptor rooted outside `workspace_dir` moved the cwd but refused every
/// read and write in it.
///
/// The grant is *additive and per-call*: it is pushed onto a clone, so nothing
/// process-global is mutated and concurrent turns cannot race each other. It
/// also cannot widen the hard invariants — `is_always_forbidden` (credential
/// stores, core OS dirs) and `is_workspace_internal_path` (core-managed state
/// under `workspace_dir`) are both evaluated *before* any trusted-root
/// shortcut, so a granted root can never expose them.
///
/// The root always originates from trusted in-process code (the session
/// builder, the sub-agent runner, or the `cwd` RPC parameter) — never from
/// model-supplied text.
pub(super) fn security_for_tool_context(
    security: &SecurityPolicy,
    context: Option<&dyn ToolRunContext>,
    tool: &str,
) -> SecurityPolicy {
    let mut scoped = security.clone();
    if let Some(workspace) = context.and_then(|ctx| ctx.workspace()) {
        tracing::debug!(
            tool,
            workspace_root = %workspace.root.display(),
            policy_id = %workspace.policy_id,
            "[tools:filesystem] granting TinyAgents workspace descriptor as action dir + trusted root"
        );
        scoped.action_dir = workspace.root.clone();
        scoped.trusted_roots.push(TrustedRoot {
            path: workspace.root.to_string_lossy().to_string(),
            access: TrustedAccess::ReadWrite,
        });
    }
    scoped
}
