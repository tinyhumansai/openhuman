//! Sandbox backend operations: backend handles and routed execution.
//!
//! Which executor runs a spawn is the security module's decision (`spawn_plan/`). This
//! module builds the request, executes exactly the plan it gets back and refuses
//! everything else.

use super::docker;
use super::shell_exec::{ComeBack, ShellRunOutcome};
use super::types::{
    ElevatedOp, SandboxBackendHandle, SandboxBackendKind, SandboxExecRequest, SandboxExecResult,
    SandboxPolicy, SandboxStatus, ELEVATED_TOOLS,
};
use crate::config::SandboxBackend;
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
/// ([`run_on_host`] and the native jail adapter): the module reads these
/// names from the parent and adds the Windows process-bootstrap variables
/// itself. Keeping the bootstrap names out of this policy prevents Windows paths
/// from being passed into Linux Docker containers.
pub const SANDBOX_ENV_PASSTHROUGH: &[&str] = &[
    "PATH", "HOME", "TERM", "LANG", "LC_ALL", "LC_CTYPE", "USER", "SHELL", "TMPDIR",
];

/// Whether a command tool must use the sandbox execution path. An explicit
/// backend in the active config is an operator requirement even when the
/// agent definition uses the usual `SandboxMode::None` default. Whether the sandbox
/// is switched off is not asked here: a sandboxed spawn reaches the module, which
/// decides if the operator's setting may skip the jail.
pub async fn command_requires_sandbox(
    ctx: &DispatchCtx,
    sandbox_mode: Option<SandboxMode>,
) -> Result<bool, String> {
    if crate::core::runtime::is_saas() {
        // Every SaaS command must enter the per-user container path, including
        // language tools whose agent definition does not request a sandbox.
        return Ok(true);
    }
    if matches!(sandbox_mode, Some(SandboxMode::Sandboxed)) {
        return Ok(true);
    }
    let config = ctx.load_config().await?;
    Ok(matches!(
        config.security.sandbox.backend,
        SandboxBackend::Landlock | SandboxBackend::Docker
    ) || config.runtime.kind == "docker")
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
    isolation: Option<super::types::SandboxIsolation>,
) -> SandboxStatus {
    if isolation.is_some() {
        SandboxStatus::Ready
    } else {
        SandboxStatus::Inactive
    }
}

/// The refusal every spawner reports when the security module planned the spawn
/// and refused it (the isolation floor, a denied grant, a human approval it
/// cannot get). A typed error so callers can tell a deterministic policy
/// refusal (never retried) from a transient failure such as a module fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IsolationFloor;

impl std::fmt::Display for IsolationFloor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the security module refused to plan this sandboxed spawn")
    }
}

impl std::error::Error for IsolationFloor {}

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
                isolation: None,
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
            isolation: None,
        }),
        SandboxBackendKind::Local => {
            let (backend_name, status, isolation) = super::native_jail::status(config).await?;
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
                isolation,
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
        isolation: None,
    }
}

/// Execute a command through the executor the security module plans for it.
///
/// Returns the execution result. The caller (typically the shell tool) is
/// responsible for converting this into a `ToolResult`.
pub async fn execute_in_sandbox(
    ctx: &DispatchCtx,
    policy: &SandboxPolicy,
    command: &str,
    working_dir: &Path,
    extra_env: HashMap<OsString, OsString>,
    timeout: Duration,
) -> anyhow::Result<SandboxExecResult> {
    execute_in_sandbox_metered(ctx, policy, command, working_dir, extra_env, timeout, None).await
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
    // The saved backend is an operator's explicit isolation requirement. It is
    // applied at the spawn boundary, including to callers that resolved their policy
    // from an older or default configuration, and goes to the module as a request.
    let config = ctx
        .load_config()
        .await
        .map_err(|e| anyhow::anyhow!("Cannot read sandbox configuration: {e}"))?;
    let policy = super::policy::with_configured_backend(policy, &config);
    let operator_off = super::operator::sandbox_off(ctx, &config);
    let admitted = admit_spawn(
        &config,
        &policy,
        command,
        working_dir,
        &extra_env,
        operator_off,
    )
    .await?;
    match admitted.executor {
        // The module planned a run on the host: the operator disabled the sandbox.
        Executor::Host => {
            crate::config::ensure_usable_cwd(working_dir)?;
            run_on_host(
                &config,
                &admitted.policy,
                command,
                working_dir,
                &extra_env,
                timeout,
                meter,
            )
            .await
        }
        Executor::Jail => {
            crate::config::ensure_usable_cwd(working_dir)?;
            super::native_jail::execute(
                &config,
                &admitted.policy,
                command,
                working_dir,
                &extra_env,
                timeout,
                admitted.jail,
                false,
                meter,
            )
            .await
        }
        Executor::Docker => {
            // The working directory is the container-side path; the host-side mount
            // source is validated instead, and only when the plan mounts it.
            if admitted.policy.mount_workspace {
                crate::config::ensure_usable_cwd(&admitted.policy.workspace_root)?;
            }
            let request = SandboxExecRequest {
                command: command.to_string(),
                working_dir: working_dir.to_path_buf(),
                env: extra_env,
                timeout,
            };
            docker::docker_exec(&config, &admitted.policy, &request).await
        }
    }
}

/// Execute an admitted spawn that may return while its native process keeps
/// running. TinySecurity remains the authority for the executor and jail
/// grants; this is only the shell tool's lifecycle adaptation.
pub(crate) async fn execute_in_sandbox_yielding(
    ctx: &DispatchCtx,
    policy: &SandboxPolicy,
    command: &str,
    working_dir: &Path,
    extra_env: HashMap<OsString, OsString>,
    timeout: Duration,
    come_back: ComeBack,
    meter: Option<super::shell_progress::ShellOutputMeter>,
) -> anyhow::Result<ShellRunOutcome> {
    let config = ctx
        .load_config()
        .await
        .map_err(|e| anyhow::anyhow!("Cannot read sandbox configuration: {e}"))?;
    let policy = super::policy::with_configured_backend(policy, &config);
    let operator_off = super::operator::sandbox_off(ctx, &config);
    let admitted = admit_spawn(
        &config,
        &policy,
        command,
        working_dir,
        &extra_env,
        operator_off,
    )
    .await?;
    match admitted.executor {
        Executor::Host => {
            crate::config::ensure_usable_cwd(working_dir)?;
            run_on_host_yielding(
                ctx,
                &config,
                &admitted.policy,
                command,
                working_dir,
                &extra_env,
                timeout,
                come_back.yield_after,
                meter,
            )
            .await
        }
        Executor::Jail => {
            crate::config::ensure_usable_cwd(working_dir)?;
            super::native_jail::execute_yielding(
                ctx,
                &config,
                &admitted.policy,
                command,
                working_dir,
                &extra_env,
                timeout,
                admitted.jail,
                false,
                come_back.yield_after,
                meter,
            )
            .await
        }
        Executor::Docker => {
            if admitted.policy.mount_workspace {
                crate::config::ensure_usable_cwd(&admitted.policy.workspace_root)?;
            }
            let timeout = come_back
                .deadline_instead
                .map_or(timeout, |deadline| timeout.min(deadline));
            let request = SandboxExecRequest {
                command: command.to_owned(),
                working_dir: working_dir.to_path_buf(),
                env: extra_env,
                timeout,
            };
            docker::docker_exec(&config, &admitted.policy, &request)
                .await
                .map(ShellRunOutcome::Finished)
        }
    }
}

/// The executor a plan selected, in the host's own terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Executor {
    /// No jail: the module planned the operator's run on the host.
    Host,
    /// The TinyBox native jail.
    Jail,
    /// A one-shot container.
    Docker,
}

/// One spawn the TinySecurity module planned and admitted: the executor it chose, the
/// policy narrowed to the plan, and the jail the plan describes for TinyBox.
#[derive(Debug)]
pub(crate) struct Admitted {
    /// The executor the plan selected.
    pub(crate) executor: Executor,
    /// The caller's policy with mounts, network and container bounds from the plan.
    pub(crate) policy: SandboxPolicy,
    /// The native jail the plan maps to; the jail executor runs exactly this.
    pub(crate) jail: super::native_jail::PlannedJail,
}

/// Have the TinySecurity module plan and admit one spawn, and return the executor, policy
/// and jail it must run under. The module's plan, not the resolved policy, bounds the
/// mounts, the network and the container limits; a refusal or a module fault is an error
/// and the command does not run.
#[cfg(feature = "security-module")]
async fn admit_spawn(
    config: &crate::config::Config,
    policy: &SandboxPolicy,
    command: &str,
    working_dir: &Path,
    env: &HashMap<OsString, OsString>,
    operator_off: bool,
) -> anyhow::Result<Admitted> {
    let env = env
        .iter()
        .map(|(name, value)| match (name.to_str(), value.to_str()) {
            (Some(name), Some(value)) => Ok((name.to_owned(), value.to_owned())),
            _ => Err(anyhow::anyhow!("sandbox environment is not UTF-8")),
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let operation = super::spawn_plan::SpawnOperation {
        command,
        cwd: working_dir,
        env,
        stdin: None,
        preserve_pipeline_status: false,
    };
    admit_planned(
        config,
        policy,
        super::spawn_plan::SpawnSource::Agent,
        &operation,
        operator_off,
    )
    .await
}

/// Plan, reserve and map one spawn.
///
/// With the operator's sandbox setting off, the module is first asked for a run on the
/// host. It grants that only to the trusted operator outside SaaS; for any other spawn
/// it refuses (`floor`) and the spawn is planned onto a real executor instead, so the
/// setting can only ever remove a jail the module agreed to remove.
///
/// A module refusal is the typed [`IsolationFloor`] (deterministic, never retried); a
/// fault stays a plain error.
#[cfg(feature = "security-module")]
pub(super) async fn admit_planned(
    config: &crate::config::Config,
    policy: &SandboxPolicy,
    source: super::spawn_plan::SpawnSource,
    operation: &super::spawn_plan::SpawnOperation<'_>,
    operator_off: bool,
) -> anyhow::Result<Admitted> {
    use super::spawn_plan::PlanError;
    use tinysecurity_bus::SandboxDenial2;

    let request =
        |policy, host_run| request_for(config, policy, source, operation, operator_off, host_run);
    if operator_off {
        match admit_request(request(policy, true)).await {
            Ok(admitted) => return Ok(admitted),
            Err(PlanError::Refused(SandboxDenial2::Floor)) => {
                tracing::debug!(
                    "[sandbox] the module refused a run on the host; planning a sandboxed spawn"
                );
            }
            Err(error) => return Err(plan_error(error)),
        }
    }
    // The toolchain and git grants the native jail needs are resolved by TinyBox. A
    // container mounts only its workspace and the policy's own read-only paths, so the
    // grants are dropped again if the module picks one.
    let granted = if policy.backend == SandboxBackendKind::Docker {
        policy.clone()
    } else {
        super::local_grants::with_resolved_grants(config, policy).await?
    };
    let mut admitted = admit_request(request(&granted, false))
        .await
        .map_err(plan_error)?;
    if admitted.executor == Executor::Docker {
        admitted
            .policy
            .read_only_mounts
            .retain(|mount| policy.read_only_mounts.contains(mount));
        admitted
            .policy
            .read_write_mounts
            .retain(|mount| policy.read_write_mounts.contains(mount));
    }
    Ok(admitted)
}

#[cfg(feature = "security-module")]
fn request_for<'a>(
    config: &'a crate::config::Config,
    policy: &'a SandboxPolicy,
    source: super::spawn_plan::SpawnSource,
    operation: &'a super::spawn_plan::SpawnOperation<'a>,
    operator_off: bool,
    host_run: bool,
) -> super::spawn_plan::SpawnRequest<'a> {
    super::spawn_plan::SpawnRequest {
        config,
        policy,
        source,
        operation,
        operator_off,
        host_run,
        module: super::spawn_plan::ModuleSource::Production,
    }
}

/// Plan one request, reserve the plan and map it to the executor the module chose.
#[cfg(feature = "security-module")]
async fn admit_request(
    request: super::spawn_plan::SpawnRequest<'_>,
) -> Result<Admitted, super::spawn_plan::PlanError> {
    let planned = super::spawn_plan::plan(request).await?;
    super::spawn_plan::consume(&planned).await?;
    Ok(Admitted {
        executor: match planned.plan.executor {
            super::plan::PlannedExecutor::Host => Executor::Host,
            super::plan::PlannedExecutor::NativeJail => Executor::Jail,
            super::plan::PlannedExecutor::Docker => Executor::Docker,
        },
        policy: planned.apply(request.policy),
        jail: super::native_jail::PlannedJail {
            config: planned.plan.jail,
        },
    })
}

#[cfg(feature = "security-module")]
fn plan_error(error: super::spawn_plan::PlanError) -> anyhow::Error {
    use super::spawn_plan::PlanError;
    match error {
        PlanError::Refused(_) => anyhow::Error::new(IsolationFloor).context(error.to_string()),
        PlanError::Unavailable(_) => anyhow::Error::new(error),
    }
}

/// Without the module there is no plan to ask for, and nothing runs unplanned.
#[cfg(not(feature = "security-module"))]
async fn admit_spawn(
    _config: &crate::config::Config,
    _policy: &SandboxPolicy,
    _command: &str,
    _working_dir: &Path,
    _env: &HashMap<OsString, OsString>,
    _operator_off: bool,
) -> anyhow::Result<Admitted> {
    anyhow::bail!("the security module is disabled; sandboxed spawns are refused")
}

/// Run a spawn on the host, exactly as the module's `host_run` plan allows.
///
/// TinyBox's `StartNativeShellExec` selects the shell (so Windows gets
/// `cmd.exe`, #4705), clears the environment down to its safe snapshot (the
/// [`SANDBOX_ENV_PASSTHROUGH`] names plus the Windows process-bootstrap
/// variables), applies the caller's overrides and owns the scratch directory.
async fn run_on_host(
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

async fn run_on_host_yielding(
    ctx: &DispatchCtx,
    config: &crate::config::Config,
    policy: &SandboxPolicy,
    command: &str,
    working_dir: &Path,
    extra_env: &HashMap<OsString, OsString>,
    timeout: Duration,
    yield_after: Option<Duration>,
    meter: Option<super::shell_progress::ShellOutputMeter>,
) -> anyhow::Result<ShellRunOutcome> {
    let env = extra_env
        .iter()
        .map(|(name, value)| match (name.to_str(), value.to_str()) {
            (Some(name), Some(value)) => Ok((name.to_owned(), value.to_owned())),
            _ => Err(anyhow::anyhow!("sandbox environment is not UTF-8")),
        })
        .collect::<anyhow::Result<_>>()?;
    std::fs::create_dir_all(&policy.state_dir)
        .map_err(|error| anyhow::anyhow!("Cannot prepare sandbox state directory: {error}"))?;
    let mut run =
        super::shell_exec::NativeShellRun::new(command, working_dir, &policy.state_dir, timeout);
    run.workspace = policy.workspace_root.clone();
    run.env = env;
    run.meter = meter;
    run.yield_after = yield_after;
    super::shell_exec::run_yielding(ctx, config, run).await
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
