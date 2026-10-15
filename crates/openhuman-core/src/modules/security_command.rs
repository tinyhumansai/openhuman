//! Host client for the module's command scopes: `RegisterCommandPolicy`,
//! `ClassifyCommand` and `CheckCommand`.
//!
//! The module classifies, gates and rate-limits shell commands; this file owns
//! only transport, the registration cache and the bridge for synchronous tool
//! traits. Settings travel as an immutable [`CommandPolicy`] derived by the
//! host from its own configuration (never from the request), and registering
//! an equal value reuses its scope, so one scope keeps one action budget for
//! its lifetime.
//!
//! * [`CommandOp::Check`] evaluates a real execution and, for an allowed
//!   command, reserves one action from the scope's hourly budget.
//! * [`CommandOp::Classify`] is informational: no budget, no admission.
//!
//! Errors deny. A faulted transport latches the shared client (no in-process
//! retry, no fallback classifier); a malformed request does not, so a hostile
//! oversize command cannot disable the module for everyone.

use std::path::Path;
use std::time::Duration;

use tinysecurity_bus::{
    names, CommandClass, CommandDecision, CommandPolicy, CommandPolicyId, CommandRequest,
};

use super::{config, require_enabled, state, ClientState};

/// Longest command text the module accepts (it refuses larger input).
pub(crate) const MAX_COMMAND_BYTES: usize = 1_048_576;
/// Registered scopes the host retains; the module caps its registry at 1024.
const MAX_SCOPES: usize = 1024;
/// A synchronous caller waits this long for a class gate before it is denied.
const BLOCKING_TIMEOUT: Duration = Duration::from_secs(3);

/// Which module member answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommandOp {
    /// `CheckCommand`: full execution check; an allowed command spends budget.
    Check,
    /// `ClassifyCommand`: class and gate only; never spends budget.
    Classify,
}

/// Check or classify `command` under `policy`, registered for the workspace
/// whose configuration binds the module.
pub(crate) async fn command_decision(
    workspace_dir: &Path,
    policy: CommandPolicy,
    command: &str,
    declared_class: Option<CommandClass>,
    op: CommandOp,
) -> Result<CommandDecision, String> {
    if command.len() > MAX_COMMAND_BYTES {
        return Err("TinySecurity command is too large".into());
    }
    let config = config::host_config(workspace_dir, crate::core::runtime::is_saas()).await?;
    require_enabled(&config)?;
    let mut state = state().lock().await;
    let result = async {
        let proxy = super::proxy(&config, &mut state).await?;
        command_on(&mut state, &proxy, policy, command, declared_class, op).await
    }
    .await;
    if let Err(error) = &result {
        log::warn!("[security][command] module call failed op={op:?}: {error}");
        state.faulted = true;
    }
    result
}

/// The registration cache plus one confidential call, shared by the production
/// path and the native-module tests.
pub(super) async fn command_on(
    state: &mut ClientState,
    proxy: &tinybus::Proxy,
    policy: CommandPolicy,
    command: &str,
    declared_class: Option<CommandClass>,
    op: CommandOp,
) -> Result<CommandDecision, String> {
    let policy_id = register_cached_command_scope(state, proxy, policy).await?;
    call_command(proxy, policy_id, command, declared_class, op).await
}

async fn register_cached_command_scope(
    state: &mut ClientState,
    proxy: &tinybus::Proxy,
    policy: CommandPolicy,
) -> Result<CommandPolicyId, String> {
    if let Some((_, id)) = state
        .commands
        .iter()
        .find(|(registered, _)| registered == &policy)
    {
        return Ok(id.clone());
    }
    if state.commands.len() >= MAX_SCOPES {
        return Err("TinySecurity host command scope limit reached".into());
    }
    let id = super::scoped_client::register_command_policy(proxy, policy.clone()).await?;
    log::debug!(
        "[security][command] registered command scope count={}",
        state.commands.len() + 1
    );
    state.commands.push((policy, id.clone()));
    Ok(id)
}

async fn call_command(
    proxy: &tinybus::Proxy,
    policy_id: CommandPolicyId,
    command: &str,
    declared_class: Option<CommandClass>,
    op: CommandOp,
) -> Result<CommandDecision, String> {
    let member = match op {
        CommandOp::Check => names::methods::CHECK_COMMAND,
        CommandOp::Classify => names::methods::CLASSIFY_COMMAND,
    };
    proxy
        .call_confidential(
            member,
            (CommandRequest {
                policy_id,
                command: command.to_owned(),
                declared_class,
            },),
        )
        .await
        .map_err(|_| match op {
            CommandOp::Check => "TinySecurity CheckCommand failed".to_owned(),
            CommandOp::Classify => "TinySecurity ClassifyCommand failed".to_owned(),
        })
}

/// [`command_decision`] for a synchronous caller (a tool trait method that
/// cannot be `async`), through the shared seam. The caller's dispatch scope is
/// captured here and re-entered on the module runtime, so a SaaS caller still
/// resolves its own tenant configuration. `Err` is a refusal.
pub(crate) fn command_decision_blocking(
    workspace_dir: &Path,
    policy: CommandPolicy,
    command: &str,
    declared_class: Option<CommandClass>,
    op: CommandOp,
) -> Result<CommandDecision, String> {
    // The module bus's own runtime serves the seam: it lives for the process,
    // so it cannot be a caller runtime that has since shut down. Until the bus
    // has started there is nothing to ask, and the call below is refused.
    super::sync_seam::arm_module_runtime();
    let scope = crate::core::runtime::DispatchCtx::ambient().core().cloned();
    let workspace_dir = workspace_dir.to_path_buf();
    let command = command.to_owned();
    super::sync_seam::run(BLOCKING_TIMEOUT, async move {
        let call = command_decision(&workspace_dir, policy, &command, declared_class, op);
        match scope {
            Some(scope) => crate::core::runtime::CoreContext::scope(scope, call).await,
            None => call.await,
        }
    })
    .map_err(String::from)
}

#[cfg(test)]
#[path = "security_command_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "security_command_characterization_tests.rs"]
mod characterization_tests;
