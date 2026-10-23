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
    /// Bytes written to the command's stdin, which is then closed. `None` gives
    /// the command no input.
    pub(crate) stdin: Option<Vec<u8>>,
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
            stdin: None,
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

/// Run `run` through the module and collect its output, waiting for the command
/// to end. This entry point also serves callers without a turn context, such as
/// cron and input-fed native operations.
pub(crate) async fn run(
    config: &Config,
    mut run: NativeShellRun,
) -> anyhow::Result<SandboxExecResult> {
    run.yield_after = None;
    #[cfg(test)]
    if !run.is_jailed() && USE_FIXTURE.load(std::sync::atomic::Ordering::SeqCst) {
        return match fixture::run(run).await? {
            ShellRunOutcome::Finished(result) => Ok(result),
            ShellRunOutcome::Running(_) => {
                anyhow::bail!("a shell run without a yield point returned while running")
            }
        };
    }
    #[cfg(feature = "modules")]
    {
        let config = config.clone();
        let meter = run.meter.take();
        let hold = tinytools_std::timeout::ProcessCleanup::hold();
        let (_caller, abandoned) = tokio::sync::oneshot::channel::<std::convert::Infallible>();
        let outcome = tokio::spawn(async move {
            let _hold = hold;
            run_owned(&config, run, abandoned, meter).await
        })
        .await
        .map_err(|_| anyhow::anyhow!("TinyBox shell worker did not finish"))??;
        match outcome {
            ShellRunOutcome::Finished(result) => Ok(result),
            ShellRunOutcome::Running(_) => {
                anyhow::bail!("a shell run without a yield point returned while running")
            }
        }
    }
    #[cfg(not(feature = "modules"))]
    {
        let _ = (config, run);
        anyhow::bail!("TinyBox module support is disabled; command execution is unavailable")
    }
}

/// Run `run` through the module. With a yield point, a command still running
/// there comes back as [`ShellRunOutcome::Running`] and keeps running.
///
/// A worker owns cancellation and resource cleanup if the caller drops its wait.
pub(crate) async fn run_yielding(
    _ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    run: NativeShellRun,
) -> anyhow::Result<ShellRunOutcome> {
    #[cfg(test)]
    if !run.is_jailed() && USE_FIXTURE.load(std::sync::atomic::Ordering::SeqCst) {
        let _ = config;
        return fixture::run(run).await;
    }
    #[cfg(feature = "modules")]
    {
        let mut run = run;
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
        let hold = tinytools_std::timeout::ProcessCleanup::hold();
        let (_caller, abandoned) = tokio::sync::oneshot::channel::<std::convert::Infallible>();
        crate::core::runtime::DispatchCtx::ambient_turn()
            .spawn(async move {
                let _hold = hold;
                run_owned(&config, run, abandoned, meter).await
            })
            .await
            .map_err(|_| anyhow::anyhow!("TinyBox shell worker did not finish"))?
    }
    #[cfg(not(feature = "modules"))]
    {
        let _ = (config, run);
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
    let resource = tinybox::reserve(config, contract::ReserveRequest::Resource)
        .await
        .map_err(|_| anyhow::anyhow!("TinyBox could not reserve a command resource"))?;
    let timeout = run.timeout;
    // A yield point only counts when it comes before the command's own
    // deadline; otherwise the run ends the usual way.
    let yield_at = run
        .yield_after
        .filter(|after| *after < timeout)
        .map(|after| tokio::time::Instant::now() + after);
    let result = async {
        tinybox::create(
            config,
            contract::CreateRequest {
                resource: resource.clone(),
                backend: "passthrough".to_owned(),
                host: contract::HostConfig::Local,
                workspace: contract::Workspace::Directory(path_string(
                    &run.workspace,
                    "command workspace",
                )?),
                network: if run.isolate_network {
                    contract::NetworkPolicy::default()
                } else {
                    contract::NetworkPolicy::Open
                },
                resources: contract::ResourceLimits::default(),
                ports: Vec::new(),
                env: BTreeMap::new(),
            },
        )
        .await
        .map_err(|_| anyhow::anyhow!("TinyBox could not create a command resource"))?;
        let process = tinybox::reserve(config, contract::ReserveRequest::Process(resource.clone()))
            .await
            .map_err(|_| anyhow::anyhow!("TinyBox could not reserve a command process"))?;
        let request = shell_exec_request(process, &resource, run)?;
        let process = tinybox::start_native_shell_exec(config, request)
            .await
            .map_err(|_| anyhow::anyhow!("TinyBox could not start the command process"))?;
        collect_process(config, process, timeout, yield_at, abandoned, meter).await
    }
    .await;
    // Still running at its yield point: the process and its resource go to
    // the caller, who releases them once it has stopped.
    let result = match result {
        Ok(Collected::Running {
            process,
            cursor,
            stdout,
            stderr,
        }) => {
            tracing::debug!("[sandbox:shell] command still running at its yield point");
            return Ok(ShellRunOutcome::Running(LiveShell {
                stdout: String::from_utf8_lossy(&stdout).into_owned(),
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
                handle: super::shell_live::LiveHandle::TinyBox(
                    super::shell_live::TinyBoxLive::new(config.clone(), resource, process, cursor),
                ),
            }));
        }
        Ok(Collected::Done(output)) => Ok(ShellRunOutcome::Finished(output)),
        Err(error) => Err(error),
    };
    let close = tinybox::close(config, resource).await;
    match (result, close) {
        (Ok(output), Ok(())) => Ok(output),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(_)) => anyhow::bail!("TinyBox did not acknowledge command cleanup"),
        (Err(error), Err(_)) => Err(error),
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
        stdin: run.stdin,
        jail: run.jail,
        hide_window: run.hide_window,
    })
}

/// What [`collect_process`] read: the whole run, or the run so far when it was
/// still going at its yield point.
#[cfg(feature = "modules")]
enum Collected {
    Done(SandboxExecResult),
    Running {
        process: contract::ProcessRef,
        cursor: u64,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
}

#[cfg(feature = "modules")]
async fn collect_process(
    config: &Config,
    process: contract::ProcessRef,
    timeout: Duration,
    yield_at: Option<tokio::time::Instant>,
    mut abandoned: tokio::sync::oneshot::Receiver<std::convert::Infallible>,
    meter: Option<super::shell_progress::ShellOutputMeter>,
) -> anyhow::Result<Collected> {
    use contract::{ExecutionState, OutputStream};
    use tokio::sync::oneshot::error::TryRecvError;

    let deadline = tokio::time::Instant::now() + timeout;
    let mut cursor = 0;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut requested_cancel = false;
    let timed_out = loop {
        let caller_gone = matches!(abandoned.try_recv(), Err(TryRecvError::Closed));
        if !requested_cancel && (caller_gone || tokio::time::Instant::now() >= deadline) {
            if caller_gone {
                tracing::debug!("[sandbox:shell] caller dropped its wait; cancelling the command");
            }
            tinybox::cancel(config, process.clone())
                .await
                .map_err(|_| {
                    anyhow::anyhow!("TinyBox could not acknowledge process cancellation")
                })?;
            requested_cancel = true;
        }
        let batch = tinybox::read_output(config, process.clone(), cursor)
            .await
            .map_err(|_| anyhow::anyhow!("TinyBox could not read command output"))?;
        if !batch.chunks.is_empty() && batch.next_sequence <= cursor {
            anyhow::bail!("TinyBox returned an invalid output cursor");
        }
        let has_chunks = !batch.chunks.is_empty();
        let state = batch.state;
        for chunk in batch.chunks {
            if let Some(meter) = &meter {
                meter.record(&chunk.bytes);
            }
            match chunk.stream {
                OutputStream::Stdout => stdout.extend_from_slice(&chunk.bytes),
                OutputStream::Stderr => stderr.extend_from_slice(&chunk.bytes),
            }
        }
        cursor = batch.next_sequence;
        match state {
            ExecutionState::Running => {
                if !requested_cancel && yield_at.is_some_and(|at| tokio::time::Instant::now() >= at)
                {
                    return Ok(Collected::Running {
                        process,
                        cursor,
                        stdout,
                        stderr,
                    });
                }
                tokio::time::sleep(Duration::from_millis(20)).await
            }
            ExecutionState::Exited { .. } if has_chunks => continue,
            ExecutionState::Exited { exit_code } => {
                tinybox::release_output(config, process)
                    .await
                    .map_err(|_| anyhow::anyhow!("TinyBox could not release command output"))?;
                return Ok(Collected::Done(SandboxExecResult {
                    exit_code,
                    stdout: String::from_utf8_lossy(&stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&stderr).into_owned(),
                    timed_out: false,
                }));
            }
            ExecutionState::Cancelled if has_chunks => continue,
            ExecutionState::Cancelled => break requested_cancel,
            ExecutionState::Failed { .. } if has_chunks => continue,
            ExecutionState::Failed { code } => {
                tracing::debug!(?code, "[sandbox:shell] TinyBox reported a failed execution");
                break failure_timed_out(code)?;
            }
        }
    };
    tinybox::release_output(config, process)
        .await
        .map_err(|_| anyhow::anyhow!("TinyBox could not release command output"))?;
    // What the command printed before it was stopped, both streams: a build
    // or test killed at its deadline has usually written its progress and
    // first errors by then. `timed_out` says why it stopped; the shell tool
    // writes the timeout line.
    Ok(Collected::Done(SandboxExecResult {
        exit_code: -1,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        timed_out,
    }))
}

/// What a terminal TinyBox failure means for the command. The module enforces
/// the same runtime budget the host waits for, so a command that runs to its
/// limit usually ends as `TimedOut` before the host's own cancel lands: that is
/// the command's timeout, reported like one (`Ok(true)`). Anything else is a
/// real sandbox failure, named by its code.
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
    config: &Config,
    command: &str,
    cwd: &std::path::Path,
    state_dir: &std::path::Path,
    env: BTreeMap<String, String>,
    timeout: Duration,
    preserve_pipeline_status: bool,
    meter: Option<super::shell_progress::ShellOutputMeter>,
) -> anyhow::Result<SandboxExecResult> {
    run_host_with_stdin(
        config,
        command,
        cwd,
        state_dir,
        env,
        timeout,
        preserve_pipeline_status,
        meter,
        None,
    )
    .await
}

/// [`run_host`] with bytes written to the command's stdin (then closed).
pub(crate) async fn run_host_with_stdin(
    config: &Config,
    command: &str,
    cwd: &std::path::Path,
    state_dir: &std::path::Path,
    env: BTreeMap<String, String>,
    timeout: Duration,
    preserve_pipeline_status: bool,
    meter: Option<super::shell_progress::ShellOutputMeter>,
    stdin: Option<Vec<u8>>,
) -> anyhow::Result<SandboxExecResult> {
    let mut shell = NativeShellRun::new(command, cwd, state_dir, timeout);
    shell.meter = meter;
    shell.env = env;
    shell.preserve_pipeline_status = preserve_pipeline_status;
    shell.stdin = stdin;
    run(config, shell).await
}

#[cfg(test)]
#[path = "shell_exec_tests.rs"]
mod tests;
