//! One shell command run through TinyBox's `StartNativeShellExec`.
//!
//! The module selects the shell (cmd, or bash with pipefail), builds the safe
//! child environment, owns the per-call scratch directory and, when asked, the
//! native jail. This file only maps a host-authorized command onto that request
//! and drives the resource, process and output lifecycle to a result.
//!
//! Without the `modules` feature there is no execution surface and every run is
//! refused; the host never falls back to spawning a shell itself.

use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use super::shell_live::LiveShell;
use super::types::SandboxExecResult;
use crate::config::Config;

#[cfg(feature = "modules")]
use crate::modules::tinybox::{self, contract};

#[cfg(test)]
#[path = "shell_exec_fixture.rs"]
pub(crate) mod fixture;

/// Everything one native shell run needs, owned so the worker can outlive the caller.
pub(crate) struct NativeShellRun {
    /// Shell text, already authorized by the host.
    pub(crate) command: String,
    /// Directory the resource is rooted at.
    pub(crate) workspace: PathBuf,
    /// Working directory of the command.
    pub(crate) cwd: PathBuf,
    /// Approved overrides on the module's safe environment snapshot.
    pub(crate) env: BTreeMap<String, String>,
    /// Existing private directory the module creates per-call scratch under.
    pub(crate) state_dir: PathBuf,
    /// Runtime budget, clamped by the module to at most twenty-four hours.
    pub(crate) timeout: Duration,
    /// Keep the shell's own last-stage pipeline status instead of `pipefail`.
    pub(crate) preserve_pipeline_status: bool,
    /// Default the Python UTF-8 variables unless the parent environment sets them.
    pub(crate) python_utf8: bool,
    /// Windows only: start the child without a console window.
    pub(crate) hide_window: bool,
    /// Whether the resource keeps default network isolation; `false` is open.
    pub(crate) isolate_network: bool,
    /// Receives the command's output as it streams, for progress reporting.
    pub(crate) meter: Option<super::shell_progress::ShellOutputMeter>,
    /// Return with the process still running once it has run this long,
    /// instead of waiting for it (the shell tool's yield point). `None`
    /// waits for the command to end or its runtime budget to stop it.
    pub(crate) yield_after: Option<Duration>,
    /// Native confinement; `None` is ordinary local execution.
    #[cfg(feature = "modules")]
    pub(crate) jail: Option<contract::NativeJailConfig>,
}

impl NativeShellRun {
    /// A host-authorized command run in `cwd` with default shell semantics.
    pub(crate) fn new(
        command: &str,
        cwd: &std::path::Path,
        state_dir: &std::path::Path,
        timeout: Duration,
    ) -> Self {
        Self {
            command: command.to_owned(),
            workspace: cwd.to_path_buf(),
            cwd: cwd.to_path_buf(),
            env: BTreeMap::new(),
            state_dir: state_dir.to_path_buf(),
            timeout,
            preserve_pipeline_status: false,
            python_utf8: false,
            hide_window: false,
            isolate_network: false,
            meter: None,
            yield_after: None,
            #[cfg(feature = "modules")]
            jail: None,
        }
    }

    #[cfg(test)]
    fn is_jailed(&self) -> bool {
        #[cfg(feature = "modules")]
        {
            self.jail.is_some()
        }
        #[cfg(not(feature = "modules"))]
        {
            false
        }
    }
}

/// Switches the unjailed path between the module and the in-process test
/// fixture. Tests do not load a native TinyBox artifact; the released-artifact
/// test turns the fixture off to drive the real bus lifecycle.
#[cfg(test)]
pub(crate) static USE_FIXTURE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);

/// How a shell call that may come back before its command ends wants a
/// sandboxed command run.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ComeBack {
    /// Return with the command still running once it has run this long. Only
    /// the native backends (no sandbox, native jail) can.
    pub(crate) yield_after: Option<Duration>,
    /// For a backend that cannot leave a command running (Docker): the
    /// caller's own come-back time, enforced as a deadline instead, as the
    /// model's `timeout_secs` was before it yielded.
    pub(crate) deadline_instead: Option<Duration>,
}

/// How a run came back: the command ended, or it was still running at its
/// yield point and is handed over to be followed.
pub(crate) enum ShellRunOutcome {
    Finished(SandboxExecResult),
    Running(LiveShell),
}

/// Run `run` through the module and collect its output, waiting for the
/// command to end (any yield point is ignored).
pub(crate) async fn run(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    mut run: NativeShellRun,
) -> anyhow::Result<SandboxExecResult> {
    run.yield_after = None;
    match run_yielding(ctx, config, run).await? {
        ShellRunOutcome::Finished(result) => Ok(result),
        ShellRunOutcome::Running(_) => {
            anyhow::bail!("a shell run without a yield point returned while running")
        }
    }
}

/// Run `run` through the module. With a yield point, a command still running
/// there comes back as [`ShellRunOutcome::Running`] and keeps running.
///
/// A worker owns cancellation and resource cleanup if the caller drops its wait.
pub(crate) async fn run_yielding(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    mut run: NativeShellRun,
) -> anyhow::Result<ShellRunOutcome> {
    #[cfg(test)]
    if !run.is_jailed() && USE_FIXTURE.load(std::sync::atomic::Ordering::SeqCst) {
        let _ = (ctx, config);
        return fixture::run(run).await;
    }
    #[cfg(feature = "modules")]
    {
        let config = config.clone();
        // Taken before the spawn: the worker task owns the meter.
        let meter = run.meter.take();
        tracing::debug!(
            jailed = run.jail.is_some(),
            timeout_ms = run.timeout.as_millis() as u64,
            yield_after_ms = run.yield_after.map(|after| after.as_millis() as u64),
            "[sandbox:shell] starting native shell through TinyBox"
        );
        // A cancelled turn drops this wait mid-command. The worker then cancels
        // the command (`_caller` closes `abandoned`) and holds the turn's
        // cleanup open until TinyBox has stopped it, so an awaited turn
        // cancellation is acknowledged only after the command is gone.
        let hold = tinyagents_harness_tools::exec::timeout::ProcessCleanup::hold();
        let (_caller, abandoned) = tokio::sync::oneshot::channel::<std::convert::Infallible>();
        ctx.spawn(async move {
            let _hold = hold;
            run_owned(&config, run, abandoned, meter).await
        })
        .await
        .map_err(|_| anyhow::anyhow!("TinyBox shell worker did not finish"))?
    }
    #[cfg(not(feature = "modules"))]
    {
        let _ = (ctx, config, run);
        anyhow::bail!("TinyBox module support is disabled; command execution is unavailable")
    }
}

#[cfg(feature = "modules")]
async fn run_owned(
    config: &Config,
    run: NativeShellRun,
    abandoned: tokio::sync::oneshot::Receiver<std::convert::Infallible>,
    meter: Option<super::shell_progress::ShellOutputMeter>,
) -> anyhow::Result<ShellRunOutcome> {
    let yield_after = run.yield_after;
    let workspace = path_string(&run.workspace, "command workspace")?;
    let request = contract::NativeShellRunRequest {
        create: contract::CreateRequest {
            resource: contract::ResourceId(String::new()),
            backend: "passthrough".to_owned(),
            host: contract::HostConfig::Local,
            workspace: contract::Workspace::Directory(workspace),
            network: if run.isolate_network {
                contract::NetworkPolicy::default()
            } else {
                contract::NetworkPolicy::Open
            },
            resources: contract::ResourceLimits::default(),
            ports: Vec::new(),
            env: BTreeMap::new(),
        },
        shell: shell_exec_request(
            contract::ResourceId(String::new()),
            &contract::ResourceId(String::new()),
            run,
        )?,
        yield_after,
    };
    let client = TinyBoxNativeShellClient { config };
    let outcome = contract::run_native_shell(
        &client,
        request,
        async move {
            let _ = abandoned.await;
        },
        |chunk| {
            if let Some(meter) = &meter {
                meter.record(&chunk.bytes);
            }
        },
    )
    .await;
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(contract::NativeShellError::Execution(code)) => {
            // v0.2.4 returns timeout output as `Finished`; this is a defensive
            // fallback for an incompatible module that reports it as a failure.
            if failure_timed_out(code)? {
                anyhow::bail!("TinyBox timed out without returning command output");
            }
            unreachable!("non-timeout execution failures return an error")
        }
        Err(error) => {
            tracing::warn!(error = ?error, "[sandbox:shell] TinyBox native shell lifecycle failed");
            anyhow::bail!("TinyBox native shell execution failed")
        }
    };
    match outcome {
        contract::NativeShellRunOutcome::Finished(output) => {
            Ok(ShellRunOutcome::Finished(SandboxExecResult {
                exit_code: output.exit_code,
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                timed_out: output.timed_out,
            }))
        }
        contract::NativeShellRunOutcome::Running(running) => {
            tracing::debug!("[sandbox:shell] command still running at its yield point");
            Ok(ShellRunOutcome::Running(LiveShell {
                stdout: String::from_utf8_lossy(&running.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&running.stderr).into_owned(),
                handle: super::shell_live::LiveHandle::TinyBox(
                    super::shell_live::TinyBoxLive::new(
                        config.clone(),
                        running.resource,
                        running.process,
                        running.cursor,
                    ),
                ),
            }))
        }
    }
}

#[cfg(feature = "modules")]
struct TinyBoxNativeShellClient<'a> {
    config: &'a Config,
}

#[cfg(feature = "modules")]
impl contract::NativeShellClient for TinyBoxNativeShellClient<'_> {
    type Error = String;

    async fn reserve(
        &self,
        request: contract::ReserveRequest,
    ) -> Result<contract::ResourceId, Self::Error> {
        tinybox::reserve(self.config, request).await
    }

    async fn create(&self, request: contract::CreateRequest) -> Result<(), Self::Error> {
        tinybox::create(self.config, request).await.map(|_| ())
    }

    async fn start_native_shell_exec(
        &self,
        request: contract::ShellExecRequest,
    ) -> Result<contract::ProcessRef, Self::Error> {
        tinybox::start_native_shell_exec(self.config, request).await
    }

    async fn read_output(
        &self,
        process: contract::ProcessRef,
        cursor: u64,
    ) -> Result<contract::OutputBatch, Self::Error> {
        tinybox::read_output(self.config, process, cursor).await
    }

    async fn cancel(&self, process: contract::ProcessRef) -> Result<(), Self::Error> {
        tinybox::cancel(self.config, process).await
    }

    async fn release_output(&self, process: contract::ProcessRef) -> Result<(), Self::Error> {
        tinybox::release_output(self.config, process).await
    }

    async fn close(&self, resource: contract::ResourceId) -> Result<(), Self::Error> {
        tinybox::close(self.config, resource).await
    }
}

/// Map a host-authorized command onto TinyBox's `StartNativeShellExec` request.
///
/// The host supplies only the command, working directory, approved environment
/// overrides and policy. Shell selection, the child environment and scratch
/// belong to the module.
#[cfg(feature = "modules")]
pub(crate) fn shell_exec_request(
    process: contract::ResourceId,
    resource: &contract::ResourceId,
    run: NativeShellRun,
) -> anyhow::Result<contract::ShellExecRequest> {
    Ok(contract::ShellExecRequest {
        process,
        resource: resource.clone(),
        shell: contract::SelectShellRequest {
            command: run.command,
            preserve_pipeline_status: run.preserve_pipeline_status,
            runtime_path: None,
            python_utf8: run.python_utf8,
        },
        cwd: Some(path_string(&run.cwd, "command working directory")?),
        extra_env: run.env,
        state_dir: path_string(&run.state_dir, "command state directory")?,
        timeout_ms: u64::try_from(run.timeout.as_millis())
            .unwrap_or(u64::MAX)
            .clamp(1, 86_400_000),
        stdin: None,
        jail: run.jail,
        hide_window: run.hide_window,
    })
}

/// Map a terminal TinyBox failure into the historical timeout/error behaviour.
#[cfg(feature = "modules")]
pub(super) fn failure_timed_out(code: contract::ExecutionFailure) -> anyhow::Result<bool> {
    use contract::ExecutionFailure;
    match code {
        ExecutionFailure::TimedOut => Ok(true),
        ExecutionFailure::CleanupFailed => {
            anyhow::bail!("TinyBox retained native process cleanup for retry")
        }
        ExecutionFailure::OutputLimit => anyhow::bail!(
            "TinyBox stopped the command: its output exceeded the execution output limit"
        ),
        ExecutionFailure::StartFailed => {
            anyhow::bail!("TinyBox native shell execution failed: the command could not start")
        }
        other => anyhow::bail!("TinyBox native shell execution failed ({other:?})"),
    }
}

#[cfg(feature = "modules")]
pub(crate) fn path_string(path: &std::path::Path, label: &str) -> anyhow::Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("{label} path is not UTF-8"))
}

/// Run a host-authorized command with the host's own network and no jail. Used
/// by cron and by the unsandboxed backend, which keep host execution semantics.
///
/// `preserve_pipeline_status` keeps a pipeline's default exit status for
/// callers whose retry or settlement behavior depends on it (cron).
pub(crate) async fn run_host(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    command: &str,
    cwd: &std::path::Path,
    state_dir: &std::path::Path,
    env: BTreeMap<String, String>,
    timeout: Duration,
    preserve_pipeline_status: bool,
    meter: Option<super::shell_progress::ShellOutputMeter>,
) -> anyhow::Result<SandboxExecResult> {
    let mut shell = NativeShellRun::new(command, cwd, state_dir, timeout);
    shell.meter = meter;
    shell.env = env;
    shell.preserve_pipeline_status = preserve_pipeline_status;
    run(ctx, config, shell).await
}

#[cfg(test)]
#[path = "shell_exec_tests.rs"]
mod tests;
