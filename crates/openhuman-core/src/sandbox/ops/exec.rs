use super::policy::{config_disables_sandbox, sandbox_disabled_by_host};
use crate::core::runtime::DispatchCtx;
use crate::sandbox::shell_exec::{ComeBack, ShellRunOutcome};
use crate::sandbox::types::{SandboxExecResult, SandboxPolicy};
#[cfg(feature = "security-module")]
use crate::sandbox::{docker, types::SandboxBackendKind, types::SandboxExecRequest};
use std::{collections::HashMap, ffi::OsString, path::Path, time::Duration};

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
    meter: Option<crate::sandbox::shell_progress::ShellOutputMeter>,
) -> anyhow::Result<SandboxExecResult> {
    let outcome = execute_in_sandbox_yielding(
        ctx,
        policy,
        command,
        working_dir,
        extra_env,
        timeout,
        ComeBack::default(),
        meter,
    )
    .await?;
    match outcome {
        ShellRunOutcome::Finished(result) => Ok(result),
        ShellRunOutcome::Running(_) => {
            anyhow::bail!("a sandboxed run without a yield point returned while running")
        }
    }
}

/// [`execute_in_sandbox_metered`] for a call that may come back first: on the
/// native backends a command still running at `come_back.yield_after` is
/// handed back as [`ShellRunOutcome::Running`]; Docker cannot leave a command
/// running, so there `come_back.deadline_instead` bounds it.
pub(crate) async fn execute_in_sandbox_yielding(
    ctx: &DispatchCtx,
    policy: &SandboxPolicy,
    command: &str,
    working_dir: &Path,
    extra_env: HashMap<OsString, OsString>,
    timeout: Duration,
    come_back: ComeBack,
    meter: Option<crate::sandbox::shell_progress::ShellOutputMeter>,
) -> anyhow::Result<ShellRunOutcome> {
    let config = ctx
        .load_config()
        .await
        .map_err(|error| anyhow::anyhow!("Cannot read sandbox configuration: {error}"))?;
    let requested_policy = policy.clone();
    let operator_off = !crate::core::runtime::is_saas()
        && (sandbox_disabled_by_host(ctx) || config_disables_sandbox(&config));
    #[cfg(feature = "security-module")]
    {
        let planned = {
            use crate::sandbox::spawn_plan::{ModuleSource, SpawnRequest, SpawnSource};
            let env = extra_env
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
                .collect::<anyhow::Result<Vec<_>>>()?;
            let operation = crate::sandbox::spawn_plan::SpawnOperation {
                command,
                cwd: working_dir,
                env,
                stdin: None,
                preserve_pipeline_status: false,
            };
            let source = SpawnSource::from_origin(ctx.turn_origin_or_scoped().as_ref());
            let host_run = operator_off && !crate::core::runtime::is_saas();
            let planned = crate::sandbox::spawn_plan::plan(SpawnRequest {
                config: &config,
                policy: &requested_policy,
                source,
                operation: &operation,
                operator_off,
                host_run,
                module: ModuleSource::Production,
            })
            .await
            .map_err(|error| anyhow::anyhow!("Sandbox plan refused: {error}"))?;
            planned
        };
        let effective_policy = planned.apply(&requested_policy);
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
                crate::sandbox::spawn_plan::consume(&planned)
                    .await
                    .map_err(|error| {
                        anyhow::anyhow!("Sandbox plan could not be consumed: {error}")
                    })?;
                execute_unsandboxed(
                    ctx,
                    &config,
                    policy,
                    command,
                    working_dir,
                    &extra_env,
                    timeout,
                    come_back.yield_after,
                    meter,
                )
                .await
            }
            SandboxBackendKind::Local => {
                crate::config::ensure_usable_cwd(working_dir)?;
                crate::sandbox::spawn_plan::consume(&planned)
                    .await
                    .map_err(|error| {
                        anyhow::anyhow!("Sandbox plan could not be consumed: {error}")
                    })?;
                crate::sandbox::native_jail::execute(
                    ctx,
                    &config,
                    policy,
                    &planned.plan.jail,
                    command,
                    working_dir,
                    &extra_env,
                    timeout,
                    false,
                    come_back.yield_after,
                    meter,
                )
                .await
            }
            SandboxBackendKind::Docker => {
                let timeout = come_back
                    .deadline_instead
                    .map_or(timeout, |deadline| timeout.min(deadline));
                crate::config::ensure_usable_cwd(&policy.workspace_root)?;
                let request = SandboxExecRequest {
                    command: command.to_string(),
                    working_dir: working_dir.to_path_buf(),
                    env: extra_env,
                    timeout,
                };
                crate::sandbox::spawn_plan::consume(&planned)
                    .await
                    .map_err(|error| {
                        anyhow::anyhow!("Sandbox plan could not be consumed: {error}")
                    })?;
                docker::docker_exec(&config, policy, &request)
                    .await
                    .map(ShellRunOutcome::Finished)
            }
            // The box has its own workspace (`/sandbox`); no host path or host
            // variable is sent, only the request's own environment. A box cannot
            // hand back a running command, so the come-back time is a deadline.
            SandboxBackendKind::OpenShell => {
                let timeout = come_back
                    .deadline_instead
                    .map_or(timeout, |deadline| timeout.min(deadline));
                crate::sandbox::spawn_plan::consume(&planned)
                    .await
                    .map_err(|error| {
                        anyhow::anyhow!("Sandbox plan could not be consumed: {error}")
                    })?;
                crate::sandbox::openshell::execute(ctx, &config, command, &extra_env, timeout)
                    .await
                    .map(ShellRunOutcome::Finished)
            }
        }
    }
    #[cfg(not(feature = "security-module"))]
    {
        let _ = (
            requested_policy,
            operator_off,
            config,
            come_back,
            meter,
            command,
            working_dir,
            extra_env,
            timeout,
        );
        anyhow::bail!("TinySecurity is not built into this host; sandboxed execution is refused")
    }
}

/// Passthrough execution with no sandbox (for `SandboxBackendKind::None`).
///
/// TinyBox's `StartNativeShellExec` selects the shell (so Windows gets
/// `cmd.exe`, #4705), clears the environment down to its safe snapshot (the
/// `SANDBOX_ENV_PASSTHROUGH` names plus the Windows process-bootstrap
/// variables), applies the caller's overrides and owns the scratch directory.
pub(crate) async fn execute_unsandboxed(
    ctx: &DispatchCtx,
    config: &crate::config::Config,
    policy: &SandboxPolicy,
    command: &str,
    working_dir: &Path,
    extra_env: &HashMap<OsString, OsString>,
    timeout: Duration,
    yield_after: Option<Duration>,
    meter: Option<crate::sandbox::shell_progress::ShellOutputMeter>,
) -> anyhow::Result<ShellRunOutcome> {
    let mut env = std::collections::BTreeMap::new();
    for (name, value) in extra_env {
        let (Some(name), Some(value)) = (name.to_str(), value.to_str()) else {
            anyhow::bail!("sandbox environment is not UTF-8");
        };
        env.insert(name.to_owned(), value.to_owned());
    }
    std::fs::create_dir_all(&policy.state_dir)
        .map_err(|error| anyhow::anyhow!("Cannot prepare sandbox state directory: {error}"))?;
    let mut run = crate::sandbox::shell_exec::NativeShellRun::new(
        command,
        working_dir,
        &policy.state_dir,
        timeout,
    );
    run.meter = meter;
    run.env = env;
    run.yield_after = yield_after;
    crate::sandbox::shell_exec::run_yielding(ctx, config, run).await
}
