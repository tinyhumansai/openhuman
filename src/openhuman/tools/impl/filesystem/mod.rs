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
use std::path::Path;
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

/// Create missing parent directories without allowing a workspace that was
/// removed after validation to be recreated by `create_dir_all`.
///
/// Workspace paths are created one component at a time from their already
/// canonical root. If that root disappears between validation and this
/// operation, creating the first missing child fails instead of silently
/// rebuilding the workspace beneath a trusted ancestor. Trusted roots that
/// are outside the workspace retain the normal recursive-create behavior.
pub(super) async fn create_validated_parent_dirs(
    policy: &SecurityPolicy,
    parent: &Path,
) -> std::io::Result<()> {
    // `validate_parent_path` may have resolved a symlinked workspace into a
    // trusted ancestor. If that workspace disappears before this helper runs,
    // checking only the raw spelling below would miss the canonical path and
    // `create_dir_all` could recreate it through the ancestor. The cached root
    // binds this operation to the workspace identity observed during
    // validation.
    if let Some(validated_root) = policy.canonical_workspace.get() {
        if parent.starts_with(validated_root) {
            match tokio::fs::canonicalize(&policy.workspace_dir).await {
                Ok(current_root) if current_root == *validated_root => {}
                _ => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "validated workspace no longer exists",
                    ));
                }
            }
        }
    }

    let workspace_root = match tokio::fs::canonicalize(&policy.workspace_dir).await {
        Ok(root) => root,
        Err(error) => {
            if parent.starts_with(&policy.workspace_dir) {
                return Err(error);
            }
            return tokio::fs::create_dir_all(parent).await;
        }
    };

    let Some(relative) = parent.strip_prefix(&workspace_root).ok() else {
        return tokio::fs::create_dir_all(parent).await;
    };

    let mut current = workspace_root;
    for component in relative.components() {
        current.push(component);
        match tokio::fs::create_dir(&current).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if !tokio::fs::metadata(&current).await?.is_dir() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        format!(
                            "validated parent component is not a directory: {}",
                            current.display()
                        ),
                    ));
                }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

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
