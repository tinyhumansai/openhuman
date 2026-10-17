//! Host policy and lifecycle adapters for TinyBox-owned native and passthrough execution.

use std::{collections::HashMap, ffi::OsString, path::Path, time::Duration};

#[cfg(feature = "modules")]
use std::collections::BTreeMap;

use super::types::{SandboxExecResult, SandboxIsolation, SandboxPolicy, SandboxStatus};
use crate::config::Config;

#[cfg(feature = "modules")]
use crate::modules::tinybox::{self, contract};

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

/// Verify that TinyBox reports a usable provider before an explicitly selected jail is accepted.
#[cfg(feature = "modules")]
pub(crate) async fn require_backend(config: &Config, expected: &str) -> anyhow::Result<()> {
    let status = tinybox::jail_status(config)
        .await
        .map_err(|_| anyhow::anyhow!("TinyBox native jail status is unavailable"))?;
    let isolation = jail_isolation(
        &status.backend,
        status.available,
        status.suitable_for_untrusted_code,
        true,
    );
    if isolation != Some(SandboxIsolation::Isolated) || status.backend != expected {
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
    ctx: &crate::core::runtime::DispatchCtx,
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
        let ctx = ctx.clone();
        let config = config.clone();
        let policy = policy.clone();
        let command = command.to_owned();
        let working_dir = working_dir.to_path_buf();
        let extra_env = extra_env.clone();
        // The worker owns cancellation and resource cleanup if the caller drops its wait.
        tokio::spawn(async move {
            execute_owned(
                &ctx,
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
            ctx,
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
    ctx: &crate::core::runtime::DispatchCtx,
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
    let untrusted = explicitly_requested || crate::core::runtime::is_saas();
    let Some(isolation) = jail_isolation(
        &status.backend,
        status.available,
        status.suitable_for_untrusted_code,
        untrusted,
    ) else {
        anyhow::bail!("TinyBox native jail cannot meet the required isolation floor");
    };
    if explicitly_requested && status.backend != "landlock" {
        anyhow::bail!("requested Landlock sandbox is unavailable");
    }
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
    run.jail = Some(jail_request(config, policy, isolation)?);
    super::shell_exec::run(ctx, config, run).await
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
fn jail_request(
    config: &Config,
    policy: &SandboxPolicy,
    isolation: SandboxIsolation,
) -> anyhow::Result<contract::NativeJailConfig> {
    let isolated = isolation == SandboxIsolation::Isolated;
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
        // Seatbelt enforces its profile best-effort; TinyBox still refuses a
        // constraint it cannot apply at all.
        enforcement: if isolated {
            contract::JailEnforcement::Enforced
        } else {
            contract::JailEnforcement::Declared
        },
        require_process_isolation: isolated,
        // Landlock denies every path it is not granted; Seatbelt reads
        // allow-default, so folder confinement names the credential stores:
        // the user's, and OpenHuman's own credential files.
        deny_read: if isolated {
            Vec::new()
        } else {
            let home =
                super::local_grants::grant_home(crate::core::runtime::is_saas(), dirs::home_dir);
            let stores = home
                .as_deref()
                .map(crate::security::policy::home_credential_stores)
                .unwrap_or_default()
                .into_iter()
                .chain(crate::security::credentials::credential_files(config))
                .collect();
            credential_denials(stores, &policy.workspace_root, &policy.read_write_mounts)?
        },
    })
}

/// The credential `stores` a folder-confined jail must not read, as existing
/// canonical paths (TinyBox drops absent ones anyway).
///
/// # Errors
///
/// Refuses the jail when its root or a writable grant sits inside a store:
/// TinyBox cannot deny a store that holds the command's workspace, and running
/// without that denial would leave the rest of the store readable. A store
/// inside a grant is fine; Seatbelt carves it out.
#[cfg(any(feature = "modules", test))]
fn credential_denials(
    stores: Vec<std::path::PathBuf>,
    root: &Path,
    read_write: &[std::path::PathBuf],
) -> anyhow::Result<Vec<String>> {
    let granted: Vec<std::path::PathBuf> = std::iter::once(root)
        .chain(read_write.iter().map(std::path::PathBuf::as_path))
        .filter_map(|path| path.canonicalize().ok())
        .collect();
    let mut denied = Vec::new();
    for store in stores {
        let Ok(real) = store.canonicalize() else {
            continue;
        };
        if granted.iter().any(|path| path.starts_with(&real)) {
            // The store's name only: the full path carries the user's login.
            let name = store
                .file_name()
                .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
            tracing::warn!(
                store = %name,
                "[sandbox:local] refusing the folder-confined jail: its workspace or a writable grant is inside a credential store"
            );
            anyhow::bail!(
                "the sandbox workspace or a writable folder is inside the credential store `{name}`; \
                 choose a working folder outside it"
            );
        }
        if let Some(real) = real.to_str() {
            denied.push(real.to_owned());
        }
    }
    Ok(denied)
}

#[cfg(feature = "modules")]
use super::shell_exec::path_string;

#[cfg(test)]
#[path = "native_jail_tests.rs"]
mod tests;
