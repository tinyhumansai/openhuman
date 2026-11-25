//! Host admission checks for asynchronous sub-agent spawns.
//!
//! Live child jobs are owned by TinyAgents' `SubAgentJobRegistry`; OpenHuman
//! keeps only the caller/workspace checks applied before it admits a spawn.

use tinyagents_orchestration::subagent::WaitError;

/// A refused detached spawn cannot be reported as accepted work.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RegistrationError {
    #[error("missing or mismatched tenant scope")]
    Scope,
}

/// Resolve the caller's profile without falling back to the operator in SaaS.
fn caller_profile(ctx: &crate::core::runtime::DispatchCtx) -> Result<Option<String>, ()> {
    let profile = ctx.tenant().map_err(|_| ())?.profile;
    if crate::core::runtime::is_saas() && profile.is_none() {
        return Err(());
    }
    Ok(profile)
}

/// Check the actual caller before any reusable-session or worker-thread write.
pub(crate) fn admit_spawn(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace: &std::path::Path,
) -> Result<Option<String>, RegistrationError> {
    admit_workspace(ctx, workspace).map_err(|_| RegistrationError::Scope)?;
    caller_profile(ctx).map_err(|_| RegistrationError::Scope)
}

/// Durable fallback paths must stay in the caller's captured workspace.
pub(super) fn admit_workspace(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace: &std::path::Path,
) -> Result<(), WaitError> {
    if !crate::core::runtime::is_saas() {
        return Ok(());
    }
    caller_profile(ctx).map_err(|_| WaitError::NotOwned)?;
    let context = ctx.resolved().ok_or(WaitError::NotOwned)?;
    if context.workspace_dir().map_err(|_| WaitError::NotOwned)? != workspace {
        return Err(WaitError::NotOwned);
    }
    Ok(())
}
