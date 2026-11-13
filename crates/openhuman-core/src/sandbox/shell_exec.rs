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

/// Run `run` through the module and collect its output.
///
/// A worker owns cancellation and resource cleanup if the caller drops its wait.
pub(crate) async fn run(
    config: &Config,
    mut run: NativeShellRun,
) -> anyhow::Result<SandboxExecResult> {
    #[cfg(test)]
    if !run.is_jailed() && USE_FIXTURE.load(std::sync::atomic::Ordering::SeqCst) {
        let _ = config;
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
            "[sandbox:shell] starting native shell through TinyBox"
        );
        // A cancelled turn drops this wait mid-command. The worker then cancels
        // the command (`_caller` closes `abandoned`) and holds the turn's
        // cleanup open until TinyBox has stopped it, so an awaited turn
        // cancellation is acknowledged only after the command is gone.
        let hold = tinytools_std::timeout::ProcessCleanup::hold();
        let (_caller, abandoned) = tokio::sync::oneshot::channel::<std::convert::Infallible>();
        crate::core::runtime::spawn_scoped(async move {
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
) -> anyhow::Result<SandboxExecResult> {
    let resource = tinybox::reserve(config, contract::ReserveRequest::Resource)
        .await
        .map_err(|_| anyhow::anyhow!("TinyBox could not reserve a command resource"))?;
    let timeout = run.timeout;
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
        collect_process(config, process, timeout, abandoned, meter).await
    }
    .await;
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
        stdin: None,
        jail: run.jail,
        hide_window: run.hide_window,
    })
}

#[cfg(feature = "modules")]
async fn collect_process(
    config: &Config,
    process: contract::ProcessRef,
    timeout: Duration,
    mut abandoned: tokio::sync::oneshot::Receiver<std::convert::Infallible>,
    meter: Option<super::shell_progress::ShellOutputMeter>,
) -> anyhow::Result<SandboxExecResult> {
    use contract::{ExecutionFailure, ExecutionState, OutputStream};
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
            ExecutionState::Running => tokio::time::sleep(Duration::from_millis(20)).await,
            ExecutionState::Exited { .. } if has_chunks => continue,
            ExecutionState::Exited { exit_code } => {
                tinybox::release_output(config, process)
                    .await
                    .map_err(|_| anyhow::anyhow!("TinyBox could not release command output"))?;
                return Ok(SandboxExecResult {
                    exit_code,
                    stdout: String::from_utf8_lossy(&stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&stderr).into_owned(),
                    timed_out: false,
                });
            }
            ExecutionState::Cancelled if has_chunks => continue,
            ExecutionState::Cancelled => break requested_cancel,
            ExecutionState::Failed {
                code: ExecutionFailure::CleanupFailed,
            } => anyhow::bail!("TinyBox retained native process cleanup for retry"),
            ExecutionState::Failed { .. } => {
                anyhow::bail!("TinyBox native shell execution failed")
            }
        }
    };
    tinybox::release_output(config, process)
        .await
        .map_err(|_| anyhow::anyhow!("TinyBox could not release command output"))?;
    Ok(SandboxExecResult {
        exit_code: -1,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: if timed_out {
            format!("Command timed out after {}s", timeout.as_secs())
        } else {
            String::from_utf8_lossy(&stderr).into_owned()
        },
        timed_out,
    })
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
    let mut shell = NativeShellRun::new(command, cwd, state_dir, timeout);
    shell.meter = meter;
    shell.env = env;
    shell.preserve_pipeline_status = preserve_pipeline_status;
    run(config, shell).await
}

#[cfg(test)]
#[path = "shell_exec_tests.rs"]
mod tests;
