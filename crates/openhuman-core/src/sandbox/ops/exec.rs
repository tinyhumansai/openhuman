use super::policy::{config_disables_sandbox, sandbox_disabled_by_host};
use crate::core::runtime::DispatchCtx;
use crate::sandbox::shell_exec::ShellRunOutcome;
use crate::sandbox::types::{SandboxBackendKind, SandboxExecResult, SandboxPolicy};
use crate::sandbox::{docker, types::SandboxExecRequest};
use std::{collections::HashMap, ffi::OsString, path::Path, time::Duration};

/// A sandbox backend resolved for a specific command, before it runs.
///
/// Resolving the backend is split from running the command so the shell can
/// learn whether the backend can leave a command running — and so decide
/// whether to reserve one of the turn's process slots — *before* it commits to
/// a run. The same resolution then drives the run, so the reservation decision
/// and the execution cannot disagree (#8317).
pub(crate) struct ResolvedSandbox {
    /// The backend the command will actually run on, after the host override,
    /// the `[components]` switch and the operator's `[sandbox]` choice.
    effective_policy: SandboxPolicy,
    config: crate::config::Config,
    /// The module's admitted plan. `None` with security disabled
    /// (`crate::security::switch`): nothing was planned, see
    /// [`unchecked_backend`].
    #[cfg(feature = "security-module")]
    planned: Option<crate::sandbox::spawn_plan::PlannedSpawn>,
}

/// The backend a command runs on when security is disabled and TinySecurity
/// does not plan the spawn: the one configured. Only the operator's switch that
/// turns the sandbox off runs it on the host; disabling security never drops a
/// requested jail or container. The native jail is then built by the host
/// ([`native_jail::host_jail`](crate::sandbox::native_jail::host_jail)).
pub(super) fn unchecked_backend(
    requested: SandboxBackendKind,
    operator_off: bool,
) -> SandboxBackendKind {
    if operator_off {
        SandboxBackendKind::None
    } else {
        requested
    }
}

impl ResolvedSandbox {
    /// The backend the command will run on.
    pub(crate) fn backend(&self) -> SandboxBackendKind {
        self.effective_policy.backend
    }

    /// Whether this backend can hand back a still-running command. Only the
    /// native backends (no sandbox, the native jail) can; Docker and the box
    /// run one-shot, so a call on them cannot yield and must not hold a slot.
    pub(crate) fn can_yield(&self) -> bool {
        matches!(
            self.backend(),
            SandboxBackendKind::None | SandboxBackendKind::Local
        )
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
    let resolved = resolve_sandbox(ctx, policy, command, working_dir, &extra_env).await?;
    let outcome = execute_resolved(
        ctx,
        resolved,
        command,
        working_dir,
        extra_env,
        timeout,
        None,
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

/// Resolve the sandbox backend for a command without running it.
///
/// Applies the host override, the `[components]` switch and the operator's
/// `[sandbox]` backend choice — everything that decides which backend a
/// command runs on — and returns it as a [`ResolvedSandbox`] the caller hands
/// straight to [`execute_resolved`]. Doing this before the caller reserves a
/// process slot is what lets a one-shot backend (Docker, the box) avoid taking
/// a slot it can never use (#8317).
pub(crate) async fn resolve_sandbox(
    ctx: &DispatchCtx,
    policy: &SandboxPolicy,
    command: &str,
    working_dir: &Path,
    extra_env: &HashMap<OsString, OsString>,
) -> anyhow::Result<ResolvedSandbox> {
    let config = ctx
        .load_config()
        .await
        .map_err(|error| anyhow::anyhow!("Cannot read sandbox configuration: {error}"))?;
    let operator_off = !crate::core::runtime::is_saas()
        && (sandbox_disabled_by_host(ctx) || config_disables_sandbox(&config));
    if crate::security::switch::disabled_for(Some(&config)).is_some() {
        let mut effective_policy = policy.clone();
        effective_policy.backend = unchecked_backend(policy.backend, operator_off);
        tracing::debug!(
            backend = ?effective_policy.backend,
            "[sandbox] security disabled; running without a TinySecurity spawn plan"
        );
        let _ = (command, working_dir, extra_env);
        return Ok(ResolvedSandbox {
            effective_policy,
            config,
            #[cfg(feature = "security-module")]
            planned: None,
        });
    }
    #[cfg(feature = "security-module")]
    {
        let requested_policy = policy.clone();
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
            let origin = ctx.turn_origin_or_scoped();
            // A unit test calls the tool directly, with no turn to label it. It
            // stands for the operator's agent; an unlabelled production call
            // stays untrusted.
            #[cfg(test)]
            let origin = origin.or(Some(
                crate::core::runtime::turn_origin::AgentTurnOrigin::Cli,
            ));
            let source = SpawnSource::from_origin(origin.as_ref());
            let host_run = operator_off && !crate::core::runtime::is_saas();
            crate::sandbox::spawn_plan::plan(SpawnRequest {
                config: &config,
                policy: &requested_policy,
                source,
                operation: &operation,
                operator_off,
                host_run,
                module: ModuleSource::Production,
            })
            .await
            .map_err(|error| anyhow::anyhow!("Sandbox plan refused: {error}"))?
        };
        let effective_policy = planned.apply(&requested_policy);
        Ok(ResolvedSandbox {
            effective_policy,
            config,
            planned: Some(planned),
        })
    }
    #[cfg(not(feature = "security-module"))]
    {
        // Unreachable: a build without the module always has security disabled.
        anyhow::bail!("TinySecurity is not built into this host; sandboxed execution is refused")
    }
}

/// Run a command on an already-[resolved](resolve_sandbox) backend.
///
/// A command still running at `yield_after` on a native backend (no sandbox,
/// the native jail) is handed back as [`ShellRunOutcome::Running`]. Docker and
/// the box run one-shot and cannot, so a call there carries no `yield_after`
/// and `timeout` is its whole bound — the caller has already reduced it to the
/// model's come-back time, as `timeout_secs` was before it yielded.
pub(crate) async fn execute_resolved(
    ctx: &DispatchCtx,
    resolved: ResolvedSandbox,
    command: &str,
    working_dir: &Path,
    extra_env: HashMap<OsString, OsString>,
    timeout: Duration,
    yield_after: Option<Duration>,
    meter: Option<crate::sandbox::shell_progress::ShellOutputMeter>,
) -> anyhow::Result<ShellRunOutcome> {
    #[cfg(feature = "security-module")]
    let resolved = match resolved {
        ResolvedSandbox {
            effective_policy,
            config,
            planned: Some(planned),
        } => (effective_policy, config, planned),
        unplanned => {
            return execute_unchecked(
                ctx,
                unplanned,
                command,
                working_dir,
                extra_env,
                timeout,
                yield_after,
                meter,
            )
            .await
        }
    };
    #[cfg(not(feature = "security-module"))]
    return execute_unchecked(
        ctx,
        resolved,
        command,
        working_dir,
        extra_env,
        timeout,
        yield_after,
        meter,
    )
    .await;
    #[cfg(feature = "security-module")]
    {
        let (effective_policy, config, planned) = resolved;
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
                    yield_after,
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
                    yield_after,
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
                crate::sandbox::spawn_plan::consume(&planned)
                    .await
                    .map_err(|error| {
                        anyhow::anyhow!("Sandbox plan could not be consumed: {error}")
                    })?;
                docker::docker_exec(&config, policy, &request)
                    .await
                    .map(ShellRunOutcome::Finished)
            }
            // The box has its own workspace (its work folder); no host path or host
            // variable is sent, only the request's own environment. A box cannot
            // hand back a running command, so the caller bounded `timeout`.
            SandboxBackendKind::OpenShell => {
                crate::sandbox::spawn_plan::consume(&planned)
                    .await
                    .map_err(|error| {
                        anyhow::anyhow!("Sandbox plan could not be consumed: {error}")
                    })?;
                crate::sandbox::boxes::execute(
                    ctx,
                    &config,
                    crate::sandbox::boxes::BoxBackend::OpenShell,
                    command,
                    &extra_env,
                    timeout,
                )
                .await
                .map(ShellRunOutcome::Finished)
            }
            SandboxBackendKind::Agent37 => {
                crate::sandbox::spawn_plan::consume(&planned)
                    .await
                    .map_err(|error| {
                        anyhow::anyhow!("Sandbox plan could not be consumed: {error}")
                    })?;
                crate::sandbox::agent37::execute(ctx, &config, command, &extra_env, timeout)
                    .await
                    .map(ShellRunOutcome::Finished)
            }
        }
    }
}

/// Run a command that TinySecurity did not plan (security disabled) on the
/// backend [`unchecked_backend`] chose: the host, a container or the box.
async fn execute_unchecked(
    ctx: &DispatchCtx,
    resolved: ResolvedSandbox,
    command: &str,
    working_dir: &Path,
    extra_env: HashMap<OsString, OsString>,
    timeout: Duration,
    yield_after: Option<Duration>,
    meter: Option<crate::sandbox::shell_progress::ShellOutputMeter>,
) -> anyhow::Result<ShellRunOutcome> {
    let ResolvedSandbox {
        effective_policy: policy,
        config,
        ..
    } = resolved;
    match policy.backend {
        SandboxBackendKind::Docker => {
            crate::config::ensure_usable_cwd(&policy.workspace_root)?;
            let request = SandboxExecRequest {
                command: command.to_string(),
                working_dir: working_dir.to_path_buf(),
                env: extra_env,
                timeout,
            };
            docker::docker_exec(&config, &policy, &request)
                .await
                .map(ShellRunOutcome::Finished)
        }
        SandboxBackendKind::OpenShell => crate::sandbox::boxes::execute(
            ctx,
            &config,
            crate::sandbox::boxes::BoxBackend::OpenShell,
            command,
            &extra_env,
            timeout,
        )
        .await
        .map(ShellRunOutcome::Finished),
        SandboxBackendKind::Agent37 => {
            crate::sandbox::agent37::execute(ctx, &config, command, &extra_env, timeout)
                .await
                .map(ShellRunOutcome::Finished)
        }
        SandboxBackendKind::Local => {
            crate::config::ensure_usable_cwd(working_dir)?;
            let policy = crate::sandbox::local_grants::with_resolved_grants(&config, &policy)
                .await
                .unwrap_or_else(|error| {
                    tracing::warn!(%error, "[sandbox] toolchain grants unavailable; the jail keeps configured grants only");
                    policy.clone()
                });
            let jail = crate::sandbox::native_jail::host_jail(&policy);
            crate::sandbox::native_jail::execute(
                ctx,
                &config,
                &policy,
                &jail,
                command,
                working_dir,
                &extra_env,
                timeout,
                false,
                yield_after,
                meter,
            )
            .await
        }
        SandboxBackendKind::None => {
            crate::config::ensure_usable_cwd(working_dir)?;
            execute_unsandboxed(
                ctx,
                &config,
                &policy,
                command,
                working_dir,
                &extra_env,
                timeout,
                yield_after,
                meter,
            )
            .await
        }
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
