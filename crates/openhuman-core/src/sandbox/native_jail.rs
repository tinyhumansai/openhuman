//! Host policy and lifecycle adapters for TinyBox-owned native and passthrough execution.

use std::{collections::HashMap, ffi::OsString, path::Path, time::Duration};

#[cfg(feature = "modules")]
use std::collections::BTreeMap;

use super::shell_exec::ShellRunOutcome;
use super::types::{SandboxIsolation, SandboxPolicy, SandboxStatus};
use crate::config::Config;

#[cfg(feature = "modules")]
use crate::modules::tinybox::contract::NativeJailConfig;
#[cfg(feature = "modules")]
use crate::modules::tinybox::{self, contract};
#[cfg(not(feature = "modules"))]
type NativeJailConfig = ();

/// TinyBox's macOS backend: `sandbox-exec` with a write-deny profile.
#[cfg(any(feature = "modules", test))]
const SEATBELT: &str = "seatbelt";

/// Display posture for the `sandbox.*` status response.
///
/// This does not admit or reject a command. TinySecurity Plan2 owns execution
/// suitability; this adapter only preserves the existing status/isolation fields.
#[cfg(any(feature = "modules", test))]
pub(crate) fn status_isolation(
    backend: &str,
    available: bool,
    suitable_for_untrusted_code: bool,
    untrusted: bool,
) -> Option<SandboxIsolation> {
    if !available {
        None
    } else if suitable_for_untrusted_code {
        Some(SandboxIsolation::Isolated)
    } else if !untrusted && backend == SEATBELT {
        Some(SandboxIsolation::FolderConfined)
    } else {
        None
    }
}

/// Report what the selected module-owned jail provides for this host.
#[cfg(feature = "modules")]
pub(crate) async fn status(
    config: &Config,
) -> Result<(String, SandboxStatus, Option<SandboxIsolation>), String> {
    let status = tinybox::jail_status(config).await?;
    let isolation = status_isolation(
        &status.backend,
        status.available,
        status.suitable_for_untrusted_code,
        crate::core::runtime::is_saas(),
    );
    Ok((
        status.backend,
        super::ops::local_status_for_jail(isolation),
        isolation,
    ))
}

#[cfg(not(feature = "modules"))]
pub(crate) async fn status(
    _config: &Config,
) -> Result<(String, SandboxStatus, Option<SandboxIsolation>), String> {
    Err("TinyBox module support is disabled; native jail status is unavailable".to_owned())
}

/// Run a command inside TinyBox's native jail and keep its resource lifecycle
/// module-owned. With `yield_after`, a command still running then comes back
/// as [`ShellRunOutcome::Running`] and keeps running in the jail.
pub(crate) async fn execute(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    policy: &SandboxPolicy,
    planned_jail: &NativeJailConfig,
    command: &str,
    working_dir: &Path,
    extra_env: &HashMap<OsString, OsString>,
    timeout: Duration,
    _explicitly_requested: bool,
    yield_after: Option<Duration>,
    meter: Option<super::shell_progress::ShellOutputMeter>,
) -> anyhow::Result<ShellRunOutcome> {
    #[cfg(feature = "modules")]
    {
        let ctx = ctx.clone();
        let config = config.clone();
        let policy = policy.clone();
        let planned_jail = planned_jail.clone();
        let command = command.to_owned();
        let working_dir = working_dir.to_path_buf();
        let extra_env = extra_env.clone();
        // The worker owns cancellation and resource cleanup if the caller drops its wait.
        tokio::spawn(async move {
            execute_owned(
                &ctx,
                &config,
                &policy,
                &planned_jail,
                &command,
                &working_dir,
                &extra_env,
                timeout,
                _explicitly_requested,
                yield_after,
                meter,
            )
            .await
        })
        .await
        .map_err(|_| anyhow::anyhow!("TinyBox native jail worker did not finish"))?
    }
    #[cfg(not(feature = "modules"))]
    {
        let _ = (
            ctx,
            config,
            policy,
            planned_jail,
            command,
            working_dir,
            extra_env,
            timeout,
            _explicitly_requested,
            yield_after,
            meter,
        );
        anyhow::bail!("TinyBox module support is disabled; native jail execution is unavailable")
    }
}

#[cfg(feature = "modules")]
async fn execute_owned(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    policy: &SandboxPolicy,
    planned_jail: &contract::NativeJailConfig,
    command: &str,
    working_dir: &Path,
    extra_env: &HashMap<OsString, OsString>,
    timeout: Duration,
    _explicitly_requested: bool,
    yield_after: Option<Duration>,
    meter: Option<super::shell_progress::ShellOutputMeter>,
) -> anyhow::Result<ShellRunOutcome> {
    let status = tinybox::jail_status(config)
        .await
        .map_err(|_| anyhow::anyhow!("TinyBox native jail status is unavailable"))?;
    if !status.available
        || (planned_jail.require_process_isolation && !status.suitable_for_untrusted_code)
    {
        anyhow::bail!("TinyBox native jail cannot meet the required isolation floor");
    }
    let isolation = if planned_jail.require_process_isolation {
        SandboxIsolation::Isolated
    } else {
        SandboxIsolation::FolderConfined
    };
    tracing::debug!(
        backend = %status.backend,
        ?isolation,
        "[sandbox:local] running in the native jail"
    );

    let mut run =
        super::shell_exec::NativeShellRun::new(command, working_dir, &policy.state_dir, timeout);
    run.workspace = policy.workspace_root.clone();
    run.env = approved_environment(extra_env)?;
    // The jail's `allow_net` is what confines the command; keep the resource's
    // declared policy in step with it.
    run.isolate_network = !policy.allow_network;
    run.meter = meter;
    // TinyBox adds its own per-call scratch directory to the writable grants and
    // points TMPDIR, TMP and TEMP at it unless the caller overrides them.
    run.jail = Some(planned_jail.clone());
    run.yield_after = yield_after;
    super::shell_exec::run_yielding(ctx, config, run).await
}

/// The caller's environment overrides, which must be UTF-8 to cross the bus.
#[cfg(feature = "modules")]
fn approved_environment(
    extra_env: &HashMap<OsString, OsString>,
) -> anyhow::Result<BTreeMap<String, String>> {
    extra_env
        .iter()
        .map(|(name, value)| {
            let name = name
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("sandbox environment name is not UTF-8"))?;
            let value = value
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("sandbox environment value is not UTF-8"))?;
            Ok((name.to_owned(), value.to_owned()))
        })
        .collect()
}

#[cfg(all(test, feature = "security-module"))]
#[path = "native_jail_tests.rs"]
mod tests;
