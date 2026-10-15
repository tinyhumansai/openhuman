//! Sandbox backend operations — policy resolution, backend creation, and
//! routed execution.

use super::docker;
use super::types::{
    ElevatedOp, SandboxBackendHandle, SandboxBackendKind, SandboxExecRequest, SandboxExecResult,
    SandboxPolicy, SandboxStatus, ELEVATED_TOOLS,
};
use crate::config::{RuntimeConfig, SandboxBackend, SandboxConfig};
use crate::core::runtime::DispatchCtx;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;
use tinyagents_definition::SandboxMode;

/// Safe environment variables forwarded into sandboxed execution.
///
/// This is the Docker container's host-environment allowlist and the set the
/// TinyBox module's own safe snapshot starts from for every native shell child
/// ([`execute_unsandboxed`] and the native jail adapter): the module reads these
/// names from the parent and adds the Windows process-bootstrap variables
/// itself. Keeping the bootstrap names out of this policy prevents Windows paths
/// from being passed into Linux Docker containers.
pub const SANDBOX_ENV_PASSTHROUGH: &[&str] = &[
    "PATH", "HOME", "TERM", "LANG", "LC_ALL", "LC_CTYPE", "USER", "SHELL", "TMPDIR",
];

/// Host switch that turns the agent sandbox off for the whole process.
///
/// For hosts that already isolate the core (a container, a CI or benchmark
/// task image, a VM): the OS jail confines commands to the action dir, which
/// breaks work such as installing packages or editing `/etc` that the outer
/// isolation already permits. `off`, `none`, `0`, `false` or `disabled`
/// (case-insensitive) disable it; anything else, or unset, leaves it on.
pub const SANDBOX_OFF_ENV: &str = "OPENHUMAN_SANDBOX";

/// Whether `value` (the `OPENHUMAN_SANDBOX` setting) disables the sandbox.
pub fn sandbox_off_value(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("off" | "none" | "0" | "false" | "disabled")
    )
}

/// Whether a command tool must use the sandbox execution path. An explicit
/// backend in the active config is an operator requirement even when the
/// agent definition uses the usual `SandboxMode::None` default.
pub async fn command_requires_sandbox(
    ctx: &DispatchCtx,
    sandbox_mode: Option<SandboxMode>,
) -> Result<bool, String> {
    if crate::core::runtime::is_saas() {
        // Every SaaS command must enter the per-user container path, including
        // language tools whose agent definition does not request a sandbox.
        return Ok(true);
    }
    if sandbox_disabled_by_host() {
        return Ok(false);
    }
    if matches!(sandbox_mode, Some(SandboxMode::Sandboxed)) {
        return Ok(true);
    }
    let config = ctx.load_config().await?;
    if config_disables_sandbox(&config) {
        return Ok(false);
    }
    Ok(matches!(
        config.sandbox.backend,
        SandboxBackend::Landlock
            | SandboxBackend::Firejail
            | SandboxBackend::Bubblewrap
            | SandboxBackend::Docker
    ))
}

/// Resolve the policy for a command tool before building its command line.
/// SaaS always uses the user's container; other hosts apply the operator's
/// backend selection while retaining an agent-level sandbox requirement.
pub(crate) async fn resolve_command_policy(
    ctx: &DispatchCtx,
    action_dir: &Path,
    state_dir: &Path,
) -> Result<SandboxPolicy, String> {
    if crate::core::runtime::is_saas() {
        return crate::profiles::tools::sandbox_policy(action_dir, state_dir);
    }
    let config = ctx
        .load_config()
        .await
        .map_err(|error| format!("Cannot read sandbox configuration: {error}"))?;
    let mut policy = resolve_sandbox_policy(
        SandboxMode::Sandboxed,
        action_dir,
        state_dir,
        &config.runtime,
        false,
    );
    if !config_disables_sandbox(&config) && !sandbox_disabled_by_host() {
        apply_requested_backend(&mut policy, &config.sandbox, &config.runtime, &config)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(policy)
}

fn sandbox_disabled_by_host() -> bool {
    sandbox_off_value(std::env::var(SANDBOX_OFF_ENV).ok().as_deref()) || sandbox_component_off()
}

/// `[components] sandbox = false` in the ambient core context. With no context
/// the component is on, since the set only narrows.
pub(crate) fn sandbox_component_off() -> bool {
    let off = crate::core::runtime::context::CoreContext::current()
        .is_some_and(|ctx| !ctx.components().sandbox);
    if off {
        tracing::debug!("[sandbox] component 'sandbox' disabled, using the unsandboxed fallback");
    }
    off
}

/// The saved config says no isolation: the legacy `enabled = false` flag or
/// `[components] sandbox = false`.
fn config_disables_sandbox(config: &crate::config::Config) -> bool {
    config.sandbox.enabled == Some(false)
        || !crate::core::runtime::ComponentSet::from_config(&config.components).sandbox
}

/// Resolve a `SandboxPolicy` from the agent's `SandboxMode`, the
/// session origin, and the global runtime config.
///
/// Non-main sessions (channel, cron, remote) default to `Docker` when
/// the mode is `Sandboxed` and Docker is configured. Local interactive
/// sessions default to `Local` (the TinyBox module's native process jail).
///
/// `state_dir` is the core's internal `workspace_dir`; host-side scratch such
/// as output capture goes there instead of into `action_dir`.
pub fn resolve_sandbox_policy(
    mode: SandboxMode,
    action_dir: &Path,
    state_dir: &Path,
    runtime_config: &RuntimeConfig,
    is_remote_session: bool,
) -> SandboxPolicy {
    let backend = match mode {
        SandboxMode::None => SandboxBackendKind::None,
        SandboxMode::ReadOnly => SandboxBackendKind::None,
        SandboxMode::Sandboxed if sandbox_disabled_by_host() => {
            tracing::debug!(
                env = SANDBOX_OFF_ENV,
                "[sandbox] host disabled the sandbox; running sandboxed mode unconfined"
            );
            SandboxBackendKind::None
        }
        SandboxMode::Sandboxed => {
            if runtime_config.kind == "docker" || is_remote_session {
                SandboxBackendKind::Docker
            } else {
                SandboxBackendKind::Local
            }
        }
    };

    let docker_overrides = if backend == SandboxBackendKind::Docker {
        let dc = &runtime_config.docker;
        Some(super::types::DockerOverrides {
            image: Some(dc.image.clone()),
            network: Some(dc.network.clone()),
            memory_limit_mb: dc.memory_limit_mb,
            cpu_limit: dc.cpu_limit,
            read_only_rootfs: Some(dc.read_only_rootfs),
            extra_caps_drop: vec![],
        })
    } else {
        None
    };

    let allow_network = match mode {
        SandboxMode::Sandboxed => !is_remote_session,
        _ => true,
    };

    // The jail denies whatever it is not told about; the toolchain/git grants
    // everyday commands need are resolved by TinyBox at spawn time
    // (`sandbox::local_grants`), so the policy starts without mounts.
    tracing::debug!(
        mode = ?mode,
        backend = ?backend,
        is_remote = is_remote_session,
        action_dir = %action_dir.display(),
        "[sandbox] resolved policy"
    );

    SandboxPolicy {
        backend,
        workspace_root: action_dir.to_path_buf(),
        state_dir: state_dir.to_path_buf(),
        read_only_mounts: Vec::new(),
        read_write_mounts: Vec::new(),
        allow_network,
        env_passthrough: SANDBOX_ENV_PASSTHROUGH
            .iter()
            .map(|s| s.to_string())
            .collect(),
        docker_overrides,
    }
}

/// Create a backend handle for the resolved policy. For Docker this
/// checks availability; for Local it checks the OS backend.
/// The status a `Local` sandbox handle reports, given the backend `pick_backend`
/// actually chose.
///
/// Extracted as a free function so the decision is testable on **any** host.
/// Asserting it through `create_sandbox_backend` cannot work: which branch runs
/// depends on whether the machine has an OS jail, so on a host with Seatbelt or
/// Landlock the noop path is never reached and a regression to "always Ready"
/// passes unnoticed. That is exactly how the original defect survived.
#[cfg(any(feature = "modules", test))]
pub(crate) fn local_status_for_jail(
    available: bool,
    suitable_for_untrusted_code: bool,
) -> SandboxStatus {
    if available && suitable_for_untrusted_code {
        SandboxStatus::Ready
    } else {
        SandboxStatus::Inactive
    }
}

/// Fail-closed gate for every spawn that runs untrusted code: a jail that is
/// missing, inactive (Noop) or not suitable for untrusted code is a denial,
/// never a silent downgrade to unsandboxed execution.
#[cfg(any(feature = "modules", test))]
pub(crate) fn require_untrusted_isolation(
    available: bool,
    suitable_for_untrusted_code: bool,
) -> Result<(), String> {
    match local_status_for_jail(available, suitable_for_untrusted_code) {
        SandboxStatus::Ready => Ok(()),
        _ => Err("the sandbox cannot meet the isolation floor for untrusted code".to_owned()),
    }
}

pub async fn create_sandbox_backend(
    ctx: &DispatchCtx,
    policy: &SandboxPolicy,
) -> SandboxBackendHandle {
    if policy.backend != SandboxBackendKind::Docker {
        return create_sandbox_backend_with_config(policy, &crate::config::Config::default())
            .await
            .unwrap_or(SandboxBackendHandle {
                kind: policy.backend,
                status: SandboxStatus::Error,
                backend_id: None,
            });
    }
    let config = match ctx.load_config().await {
        Ok(config) => config,
        Err(_) if policy.backend == SandboxBackendKind::Docker => {
            return docker_error_handle();
        }
        Err(_) => crate::config::Config::default(),
    };
    create_sandbox_backend_with_config(policy, &config)
        .await
        .unwrap_or_else(|_| docker_error_handle())
}

/// Resolve backend status with the caller's loaded configuration.
pub async fn create_sandbox_backend_with_config(
    policy: &SandboxPolicy,
    config: &crate::config::Config,
) -> Result<SandboxBackendHandle, String> {
    match policy.backend {
        SandboxBackendKind::None => Ok(SandboxBackendHandle {
            kind: SandboxBackendKind::None,
            status: SandboxStatus::Ready,
            backend_id: None,
        }),
        SandboxBackendKind::Local => {
            let (backend_name, status) = super::native_jail::status(config).await?;
            if status == SandboxStatus::Inactive {
                tracing::warn!(
                    backend = backend_name,
                    "[sandbox:local] TinyBox native jail is unavailable or below the required isolation floor"
                );
            }
            Ok(SandboxBackendHandle {
                kind: SandboxBackendKind::Local,
                status,
                backend_id: Some(backend_name),
            })
        }
        SandboxBackendKind::Docker => docker::docker_backend_handle(config).await,
    }
}

fn docker_error_handle() -> SandboxBackendHandle {
    SandboxBackendHandle {
        kind: SandboxBackendKind::Docker,
        status: SandboxStatus::Error,
        backend_id: None,
    }
}

/// Execute a command through the appropriate sandbox backend.
///
/// Returns the sandboxed execution result. The caller (typically the
/// shell tool) is responsible for converting this into a `ToolResult`.
pub async fn execute_in_sandbox(
    ctx: &DispatchCtx,
    policy: &SandboxPolicy,
    command: &str,
    working_dir: &Path,
    extra_env: HashMap<OsString, OsString>,
    timeout: Duration,
) -> anyhow::Result<SandboxExecResult> {
    execute_in_sandbox_metered(&ctx, policy, command, working_dir, extra_env, timeout, None).await
}

/// [`execute_in_sandbox`] that feeds `meter` with the command's output as it
/// streams (the native backends; Docker reports elapsed time only), for the
/// shell tool's progress line.
pub(crate) async fn execute_in_sandbox_metered(
    ctx: &DispatchCtx,
    policy: &SandboxPolicy,
    command: &str,
    working_dir: &Path,
    extra_env: HashMap<OsString, OsString>,
    timeout: Duration,
    meter: Option<super::shell_progress::ShellOutputMeter>,
) -> anyhow::Result<SandboxExecResult> {
    // The saved backend is an operator's explicit isolation requirement. It
    // must be checked at the spawn boundary, including callers that resolved
    // their policy from an older or default RuntimeConfig.
    let mut effective_policy = policy.clone();
    if !crate::core::runtime::is_saas() && sandbox_disabled_by_host() {
        effective_policy.backend = SandboxBackendKind::None;
    }
    let mut module_config = None;
    let explicitly_requested = if crate::core::runtime::is_saas() || sandbox_disabled_by_host() {
        false
    } else {
        let config = ctx
            .load_config()
            .await
            .map_err(|e| anyhow::anyhow!("Cannot read sandbox configuration: {e}"))?;
        module_config = Some(config.clone());
        if config_disables_sandbox(&config) {
            false
        } else {
            apply_requested_backend(
                &mut effective_policy,
                &config.sandbox,
                &config.runtime,
                &config,
            )
            .await?
        }
    };
    let policy = &effective_policy;
    // Validate the working directory up front so a missing/bad action_dir
    // surfaces an actionable, path-naming error here rather than an opaque OS
    // error 267 (ERROR_DIRECTORY) at spawn time — parity with the unsandboxed
    // `NativeRuntime::build_shell_command` guard. (#3353, Fix 2)
    //
    // The validation is host-side, so it is applied per-backend: for None/Local
    // `working_dir` *is* a host path; for Docker `working_dir` is the
    // container-side mount target (e.g. `/workspace`) which must NOT be
    // stat'd/created on the host — there we validate the host-side mount source
    // (`policy.workspace_root`) instead.
    match policy.backend {
        SandboxBackendKind::None => {
            crate::config::ensure_usable_cwd(working_dir)?;
            let config = match module_config {
                Some(config) => config,
                None => ctx.load_config().await.map_err(|error| {
                    anyhow::anyhow!("Cannot read module configuration: {error}")
                })?,
            };
            execute_unsandboxed(
                &config,
                policy,
                command,
                working_dir,
                &extra_env,
                timeout,
                meter,
            )
            .await
        }
        SandboxBackendKind::Local => {
            crate::config::ensure_usable_cwd(working_dir)?;
            let config = match module_config {
                Some(config) => config,
                None => ctx.load_config().await.map_err(|error| {
                    anyhow::anyhow!("Cannot read module configuration: {error}")
                })?,
            };
            let policy = super::local_grants::with_resolved_grants(&config, policy).await?;
            super::native_jail::execute(
                &config,
                &policy,
                command,
                working_dir,
                &extra_env,
                timeout,
                explicitly_requested,
                meter,
            )
            .await
        }
        SandboxBackendKind::Docker => {
            crate::config::ensure_usable_cwd(&policy.workspace_root)?;
            let request = SandboxExecRequest {
                command: command.to_string(),
                working_dir: working_dir.to_path_buf(),
                env: extra_env,
                timeout,
            };
            let config = match module_config {
                Some(config) => config,
                None => ctx.load_config().await.map_err(|error| {
                    anyhow::anyhow!("Cannot read module configuration: {error}")
                })?,
            };
            docker::docker_exec(&config, policy, &request).await
        }
    }
}

/// Apply an operator-selected backend. Auto retains the caller's existing
/// policy. None opts out only when no agent-level sandbox is required.
pub(crate) async fn apply_requested_backend(
    policy: &mut SandboxPolicy,
    sandbox: &SandboxConfig,
    runtime: &RuntimeConfig,
    config: &crate::config::Config,
) -> anyhow::Result<bool> {
    match &sandbox.backend {
        SandboxBackend::Auto => Ok(false),
        SandboxBackend::None => Ok(false),
        SandboxBackend::Docker => {
            if policy.backend == SandboxBackendKind::Local {
                policy.read_only_mounts.clear();
                policy.read_write_mounts.clear();
            }
            policy.backend = SandboxBackendKind::Docker;
            let dc = &runtime.docker;
            policy.docker_overrides = Some(super::types::DockerOverrides {
                image: Some(dc.image.clone()),
                network: Some(dc.network.clone()),
                memory_limit_mb: dc.memory_limit_mb,
                cpu_limit: dc.cpu_limit,
                read_only_rootfs: Some(dc.read_only_rootfs),
                extra_caps_drop: vec![],
            });
            Ok(true)
        }
        SandboxBackend::Landlock => {
            super::native_jail::require_backend(config, "landlock").await?;
            policy.backend = SandboxBackendKind::Local;
            policy.docker_overrides = None;
            Ok(true)
        }
        SandboxBackend::Firejail => {
            anyhow::bail!(
                "Firejail sandbox was explicitly requested but this build has no Firejail execution backend"
            )
        }
        SandboxBackend::Bubblewrap => {
            anyhow::bail!(
                "Bubblewrap sandbox was explicitly requested but this build has no Bubblewrap execution backend"
            )
        }
    }
}

/// Passthrough execution with no sandbox (for `SandboxBackendKind::None`).
///
/// TinyBox's `StartNativeShellExec` selects the shell (so Windows gets
/// `cmd.exe`, #4705), clears the environment down to its safe snapshot (the
/// [`SANDBOX_ENV_PASSTHROUGH`] names plus the Windows process-bootstrap
/// variables), applies the caller's overrides and owns the scratch directory.
async fn execute_unsandboxed(
    config: &crate::config::Config,
    policy: &SandboxPolicy,
    command: &str,
    working_dir: &Path,
    extra_env: &HashMap<OsString, OsString>,
    timeout: Duration,
    meter: Option<super::shell_progress::ShellOutputMeter>,
) -> anyhow::Result<SandboxExecResult> {
    let mut env = std::collections::BTreeMap::new();
    for (name, value) in extra_env {
        let (Some(name), Some(value)) = (name.to_str(), value.to_str()) else {
            anyhow::bail!("sandbox environment is not UTF-8");
        };
        env.insert(name.to_owned(), value.to_owned());
    }
    std::fs::create_dir_all(&policy.state_dir)
        .map_err(|error| anyhow::anyhow!("Cannot prepare sandbox state directory: {error}"))?;
    super::shell_exec::run_host(
        config,
        command,
        working_dir,
        &policy.state_dir,
        env,
        timeout,
        false,
        meter,
    )
    .await
}

/// Check whether a tool operation is an elevated op that must run on the
/// host even when the session is sandboxed.
pub fn is_elevated_op(tool_name: &str) -> bool {
    ELEVATED_TOOLS.contains(&tool_name)
}

/// Build an `ElevatedOp` for audit logging when a tool bypasses the sandbox.
pub fn build_elevated_op(tool_name: &str, command: &str, reason: &str) -> ElevatedOp {
    tracing::info!(
        tool = tool_name,
        reason = reason,
        "[sandbox] elevated host operation"
    );
    ElevatedOp {
        tool_name: tool_name.to_string(),
        reason: reason.to_string(),
        command: command.to_string(),
    }
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
