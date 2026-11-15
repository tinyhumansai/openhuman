//! Shell tool behaviour that differs in SaaS mode, split out so each branch can
//! be tested with the mode passed in.

use tinytools::ToolResult;

/// In SaaS, resolves the container policy every command must run in; `None`
/// outside SaaS, where the agent's sandbox mode decides.
pub(crate) fn saas_sandbox_with(
    saas: bool,
    resolve: impl FnOnce() -> Result<crate::sandbox::SandboxPolicy, String>,
) -> Option<Result<crate::sandbox::SandboxPolicy, String>> {
    saas.then(resolve)
}

/// The result for a SaaS command whose sandbox could not be set up: nothing
/// ran, so it is reported as not allowed, and never falls back to the host.
pub(crate) fn saas_sandbox_refusal(why: &str) -> (bool, ToolResult) {
    (
        false,
        ToolResult::error(format!("Sandbox unavailable: {why}")),
    )
}

#[cfg(test)]
#[path = "shell_saas_tests.rs"]
mod tests;
