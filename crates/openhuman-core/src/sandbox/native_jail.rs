//! Host policy and lifecycle adapters for TinyBox-owned native and passthrough execution.

use std::{collections::HashMap, ffi::OsString, path::Path, time::Duration};

#[cfg(feature = "modules")]
use std::collections::BTreeMap;

use super::types::{SandboxExecResult, SandboxIsolation, SandboxPolicy, SandboxStatus};
use crate::config::Config;

#[cfg(feature = "modules")]
use crate::modules::tinybox::{self, contract};

/// The jail the module planned for one spawn, in TinyBox's vocabulary. The host
/// never derives grants, network or enforcement of its own.
#[cfg(feature = "modules")]
#[derive(Debug)]
pub(crate) struct PlannedJail {
    /// Planned grants and confinement requirements.
    pub(crate) config: contract::NativeJailConfig,
}
/// Without the module there is no plan to carry.
#[cfg(not(feature = "modules"))]
#[derive(Debug)]
pub(crate) struct PlannedJail;

/// TinyBox's macOS backend: `sandbox-exec` with a write-deny profile.
#[cfg(any(feature = "modules", test))]
const SEATBELT: &str = "seatbelt";

/// The confinement a native jail gives a command, or `None` when the host must
/// refuse to run it.
///
/// Untrusted code (an explicitly requested Landlock jail, a SaaS tenant) needs
/// kernel process isolation. The local desktop sandbox only has to keep the
/// agent inside the selected folder, which Seatbelt does by denying writes
/// everywhere else, so it is accepted there as folder confinement.
#[cfg(any(feature = "modules", test))]
pub(crate) fn jail_isolation(
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
    let isolation = jail_isolation(
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

/// Run a command inside TinyBox's native jail and keep its resource lifecycle module-owned.
pub(crate) async fn execute(
    config: &Config,
    policy: &SandboxPolicy,
    command: &str,
    working_dir: &Path,
    extra_env: &HashMap<OsString, OsString>,
    timeout: Duration,
    jail: PlannedJail,
    preserve_pipeline_status: bool,
    meter: Option<super::shell_progress::ShellOutputMeter>,
) -> anyhow::Result<SandboxExecResult> {
    execute_with_stdin(
        config,
        policy,
        command,
        working_dir,
        extra_env,
        timeout,
        jail,
        preserve_pipeline_status,
        meter,
        None,
    )
    .await
}

/// [`execute`] with bytes written to the command's stdin (then closed).
pub(crate) async fn execute_with_stdin(
    config: &Config,
    policy: &SandboxPolicy,
    command: &str,
    working_dir: &Path,
    extra_env: &HashMap<OsString, OsString>,
    timeout: Duration,
    jail: PlannedJail,
    preserve_pipeline_status: bool,
    meter: Option<super::shell_progress::ShellOutputMeter>,
    stdin: Option<Vec<u8>>,
) -> anyhow::Result<SandboxExecResult> {
    #[cfg(feature = "modules")]
    {
        let config = config.clone();
        let policy = policy.clone();
        let command = command.to_owned();
        let working_dir = working_dir.to_path_buf();
        let extra_env = extra_env.clone();
        // The worker owns cancellation and resource cleanup if the caller drops its wait.
        tokio::spawn(async move {
            execute_owned(
                &config,
                &policy,
                &command,
                &working_dir,
                &extra_env,
                timeout,
                jail,
                preserve_pipeline_status,
                meter,
                stdin,
            )
            .await
        })
        .await
        .map_err(|_| anyhow::anyhow!("TinyBox native jail worker did not finish"))?
    }
    #[cfg(not(feature = "modules"))]
    {
        let _ = (
            config,
            policy,
            command,
            working_dir,
            extra_env,
            timeout,
            jail,
            preserve_pipeline_status,
            meter,
            stdin,
        );
        anyhow::bail!("TinyBox module support is disabled; native jail execution is unavailable")
    }
}

#[cfg(feature = "modules")]
async fn execute_owned(
    config: &Config,
    policy: &SandboxPolicy,
    command: &str,
    working_dir: &Path,
    extra_env: &HashMap<OsString, OsString>,
    timeout: Duration,
    jail: PlannedJail,
    preserve_pipeline_status: bool,
    meter: Option<super::shell_progress::ShellOutputMeter>,
    stdin: Option<Vec<u8>>,
) -> anyhow::Result<SandboxExecResult> {
    tracing::debug!(
        allow_net = jail.config.allow_net,
        require_process_isolation = jail.config.require_process_isolation,
        "[sandbox:local] running in the native jail the module planned"
    );

    let mut run =
        super::shell_exec::NativeShellRun::new(command, working_dir, &policy.state_dir, timeout);
    run.workspace = policy.workspace_root.clone();
    run.env = approved_environment(extra_env)?;
    // The jail's `allow_net` is what confines the command; keep the resource's
    // declared policy in step with it.
    run.isolate_network = !jail.config.allow_net;
    run.preserve_pipeline_status = preserve_pipeline_status;
    run.meter = meter;
    run.stdin = stdin;
    // TinyBox adds its own per-call scratch directory to the writable grants and
    // points TMPDIR, TMP and TEMP at it unless the caller overrides them. The
    // grants, network, subprocess rule and isolation requirement are the
    // module's plan, mapped by `sandbox::plan`, not derived here.
    run.jail = Some(jail.config);
    super::shell_exec::run(config, run).await
}

/// Execute a native jail plan that may yield while the command continues.
/// The confinement settings come from TinySecurity's admitted plan; this
/// adapter only maps those settings and hands the process lifecycle to TinyBox.
pub(crate) async fn execute_yielding(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    policy: &SandboxPolicy,
    command: &str,
    working_dir: &Path,
    extra_env: &HashMap<OsString, OsString>,
    timeout: Duration,
    jail: PlannedJail,
    preserve_pipeline_status: bool,
    yield_after: Option<Duration>,
    meter: Option<super::shell_progress::ShellOutputMeter>,
) -> anyhow::Result<super::shell_exec::ShellRunOutcome> {
    let mut run =
        super::shell_exec::NativeShellRun::new(command, working_dir, &policy.state_dir, timeout);
    run.workspace = policy.workspace_root.clone();
    run.env = approved_environment(extra_env)?;
    run.isolate_network = !jail.config.allow_net;
    run.preserve_pipeline_status = preserve_pipeline_status;
    run.meter = meter;
    run.yield_after = yield_after;
    run.jail = Some(jail.config);
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

#[cfg(test)]
#[path = "native_jail_tests.rs"]
mod tests;
