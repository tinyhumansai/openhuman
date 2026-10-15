//! Host policy and lifecycle adapters for TinyBox-owned native and passthrough execution.

use std::{collections::HashMap, ffi::OsString, path::Path, time::Duration};

#[cfg(feature = "modules")]
use std::collections::BTreeMap;

use super::types::{SandboxExecResult, SandboxPolicy, SandboxStatus};
use crate::config::Config;

#[cfg(feature = "modules")]
use crate::modules::tinybox::{self, contract};

/// Report whether the selected module-owned jail can meet the host's security floor.
#[cfg(feature = "modules")]
pub(crate) async fn status(config: &Config) -> Result<(String, SandboxStatus), String> {
    let status = tinybox::jail_status(config).await?;
    Ok((
        status.backend,
        super::ops::local_status_for_jail(status.available, status.suitable_for_untrusted_code),
    ))
}

#[cfg(not(feature = "modules"))]
pub(crate) async fn status(_config: &Config) -> Result<(String, SandboxStatus), String> {
    Err("TinyBox module support is disabled; native jail status is unavailable".to_owned())
}

/// Verify that TinyBox reports a usable provider before an explicitly selected jail is accepted.
#[cfg(feature = "modules")]
pub(crate) async fn require_backend(config: &Config, expected: &str) -> anyhow::Result<()> {
    let status = tinybox::jail_status(config)
        .await
        .map_err(|_| anyhow::anyhow!("TinyBox native jail status is unavailable"))?;
    if !status.available || !status.suitable_for_untrusted_code || status.backend != expected {
        anyhow::bail!("requested native jail backend is unavailable");
    }
    Ok(())
}

#[cfg(not(feature = "modules"))]
pub(crate) async fn require_backend(_config: &Config, _expected: &str) -> anyhow::Result<()> {
    anyhow::bail!("TinyBox module support is disabled; native jail execution is unavailable")
}

/// Run a command inside TinyBox's native jail and keep its resource lifecycle module-owned.
pub(crate) async fn execute(
    config: &Config,
    policy: &SandboxPolicy,
    command: &str,
    working_dir: &Path,
    extra_env: &HashMap<OsString, OsString>,
    timeout: Duration,
    explicitly_requested: bool,
    meter: Option<super::shell_progress::ShellOutputMeter>,
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
                explicitly_requested,
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
            config,
            policy,
            command,
            working_dir,
            extra_env,
            timeout,
            explicitly_requested,
            meter,
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
    explicitly_requested: bool,
    meter: Option<super::shell_progress::ShellOutputMeter>,
) -> anyhow::Result<SandboxExecResult> {
    let status = tinybox::jail_status(config)
        .await
        .map_err(|_| anyhow::anyhow!("TinyBox native jail status is unavailable"))?;
    super::ops::require_untrusted_isolation(status.available, status.suitable_for_untrusted_code)
        .map_err(|error| anyhow::anyhow!(error))?;
    if explicitly_requested && status.backend != "landlock" {
        anyhow::bail!("requested Landlock sandbox is unavailable");
    }

    let mut run =
        super::shell_exec::NativeShellRun::new(command, working_dir, &policy.state_dir, timeout);
    run.workspace = policy.workspace_root.clone();
    run.env = approved_environment(extra_env)?;
    run.isolate_network = true;
    run.meter = meter;
    // TinyBox adds its own per-call scratch directory to the writable grants and
    // points TMPDIR, TMP and TEMP at it unless the caller overrides them.
    run.jail = Some(jail_request(policy)?);
    super::shell_exec::run(config, run).await
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

#[cfg(feature = "modules")]
fn jail_request(policy: &SandboxPolicy) -> anyhow::Result<contract::NativeJailConfig> {
    Ok(contract::NativeJailConfig {
        read_only: policy
            .read_only_mounts
            .iter()
            .map(|path| path_string(path, "read-only grant"))
            .collect::<anyhow::Result<Vec<_>>>()?,
        read_write: policy
            .read_write_mounts
            .iter()
            .map(|path| path_string(path, "read-write grant"))
            .collect::<anyhow::Result<Vec<_>>>()?,
        allow_net: policy.allow_network,
        allow_subprocess: true,
        enforcement: contract::JailEnforcement::Enforced,
        require_process_isolation: true,
    })
}

#[cfg(feature = "modules")]
use super::shell_exec::path_string;

#[cfg(test)]
#[path = "native_jail_tests.rs"]
mod tests;
