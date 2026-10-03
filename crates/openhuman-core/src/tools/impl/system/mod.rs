//! Cross-cutting system tools: shell, Node/npm/Python execution, tool
//! detection/install, time helpers, LSP, proxy config, update check/apply.
//! Each family, its tool structs, and its registration gates are catalogued in
//! `tools/impl/README.md`; `security_for_tool_context` below is the shared
//! `SecurityPolicy` resolver that must stay in step with the `filesystem` copy.

mod install_tool;
mod lsp;
mod node_exec;
mod npm_exec;
mod proxy_config;
mod python_exec;
mod retrieve_tool_output;
mod schedule;
mod shell;
mod update_apply;
mod update_check;

use crate::security::policy::{TrustedAccess, TrustedRoot};
use crate::security::SecurityPolicy;
use tinytools::ToolRunContext;

pub use install_tool::InstallToolTool;
pub use lsp::{lsp_capability_enabled, LspTool, LSP_ENABLED_ENV};
pub use node_exec::NodeExecTool;
pub use npm_exec::NpmExecTool;
pub use proxy_config::ProxyConfigTool;
pub use python_exec::PythonExecTool;
pub use retrieve_tool_output::{retrieve_tool_output_tool, RetrieveToolOutputTool};
pub use schedule::ScheduleTool;
pub use shell::ShellTool;
pub use update_apply::UpdateApplyTool;
pub use update_check::UpdateCheckTool;

/// Clone `security` and scope it to the run's workspace descriptor, if any.
///
/// The process-tool counterpart of
/// `tools/impl/filesystem/gate.rs` (`security_scoped_to_root`), and it must stay in step
/// with it: the descriptor's root becomes both the relative-path resolution
/// root (`action_dir`) **and** a `ReadWrite` trusted root. The grant is the
/// load-bearing half — `action_dir` only decides where a relative path lands,
/// while the allow/deny decision reads `workspace_dir` + `trusted_roots`
/// (`SecurityPolicy::is_resolved_path_allowed_for`). Granting it in the
/// filesystem copy alone would let an agent read and edit a checkout it could
/// not then build, test, or commit, because `shell`, `python_exec`,
/// `node_exec`, and `npm_exec` all resolve their paths through here.
///
/// The grant is *additive and per-call*: it is pushed onto a clone, so nothing
/// process-global is mutated and concurrent turns cannot race each other. It
/// cannot widen the hard invariants either — `is_always_forbidden` and
/// `is_workspace_internal_path` are both evaluated *before* any trusted-root
/// shortcut.
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
            "[tools:system] granting TinyAgents workspace descriptor as action dir + trusted root"
        );
        scoped.action_dir = workspace.root.clone();
        scoped.trusted_roots.push(TrustedRoot {
            path: workspace.root.to_string_lossy().to_string(),
            access: TrustedAccess::ReadWrite,
        });
    }
    scoped
}
