//! A shell command still running after its call returned.
//!
//! The `shell` tool returns at its yield point instead of waiting out the
//! command, and leaves the process running. [`LiveShell`] is what it hands over:
//! the output read so far and a [`LiveHandle`] that reads more, stops the
//! process, and releases it. TinyBox already keeps a process running and its
//! output journalled whether or not the host reads it; this only stops the host
//! from cancelling it when the call returns.
//!
//! A handle that is dropped while its process still runs (the turn ended)
//! stops the process and releases its resource in the background.

use std::time::Duration;

#[cfg(feature = "modules")]
use crate::modules::tinybox::{self, contract};

/// Pause between output reads while a process runs.
#[cfg(feature = "modules")]
const READ_INTERVAL: Duration = Duration::from_millis(50);

/// How long [`LiveHandle::kill`] waits for the stopped process to report.
const KILL_GRACE: Duration = Duration::from_secs(10);

/// The output a process wrote so far, at its yield point.
pub(crate) struct LiveShell {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) handle: LiveHandle,
}

/// Where a process is after a read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LiveState {
    /// Still running.
    Running,
    /// Exited on its own with this code.
    Exited(i32),
    /// Stopped at its runtime budget (`timeout_secs`, clamped to the turn).
    TimedOut,
    /// Stopped by the model or the host.
    Stopped,
}

/// Output written since the previous read, and where the process is now.
pub(crate) struct LiveRead {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) state: LiveState,
}

/// A running process the host can read, stop and release.
pub(crate) enum LiveHandle {
    #[cfg(feature = "modules")]
    TinyBox(TinyBoxLive),
    #[cfg(test)]
    Fixture(super::shell_exec::fixture::FixtureLive),
}

impl LiveHandle {
    /// Read until the process ends or `wait` passes, whichever is first.
    pub(crate) async fn read(&mut self, wait: Duration) -> anyhow::Result<LiveRead> {
        // `*self`, not `self`: without `modules` (outside tests) the enum has no
        // variants, and only a match on the place itself is exhaustive then.
        match *self {
            #[cfg(feature = "modules")]
            Self::TinyBox(ref mut live) => live.read(wait).await,
            #[cfg(test)]
            Self::Fixture(ref mut live) => live.read(wait).await,
        }
    }

    /// Whether the process has stopped and its resource is released, so a
    /// failed read leaves nothing running.
    pub(crate) fn finished(&self) -> bool {
        match *self {
            #[cfg(feature = "modules")]
            Self::TinyBox(ref live) => live.released,
            #[cfg(test)]
            Self::Fixture(ref live) => live.finished(),
        }
    }

    /// Stop the process and return what it wrote since the previous read.
    pub(crate) async fn kill(&mut self) -> anyhow::Result<LiveRead> {
        match *self {
            #[cfg(feature = "modules")]
            Self::TinyBox(ref mut live) => live.kill().await,
            #[cfg(test)]
            Self::Fixture(ref mut live) => live.kill().await,
        }
    }
}

/// A TinyBox native shell process past its yield point. Owns the command's
/// resource until the process has stopped and its output is released.
#[cfg(feature = "modules")]
pub(crate) struct TinyBoxLive {
    config: crate::config::Config,
    resource: contract::ResourceId,
    process: contract::ProcessRef,
    cursor: u64,
    released: bool,
}

#[cfg(feature = "modules")]
impl TinyBoxLive {
    pub(crate) fn new(
        config: crate::config::Config,
        resource: contract::ResourceId,
        process: contract::ProcessRef,
        cursor: u64,
    ) -> Self {
        Self {
            config,
            resource,
            process,
            cursor,
            released: false,
        }
    }

    async fn read(&mut self, wait: Duration) -> anyhow::Result<LiveRead> {
        let until = tokio::time::Instant::now() + wait;
        self.read_until(until).await
    }

    /// Cancel the process and read until it reports how it ended. One still
    /// running after [`KILL_GRACE`] comes back as `Running`: the cancel did
    /// not take, and the process keeps its place until it does stop.
    async fn kill(&mut self) -> anyhow::Result<LiveRead> {
        if !self.released {
            tracing::debug!("[sandbox:shell] stopping a running process on request");
            // A process that ended on its own refuses the cancel; the read
            // below reports how it ended either way.
            if tinybox::cancel(&self.config, self.process.clone())
                .await
                .is_err()
            {
                tracing::debug!("[sandbox:shell] TinyBox refused the cancel");
            }
        }
        let read = self
            .read_until(tokio::time::Instant::now() + KILL_GRACE)
            .await?;
        if read.state == LiveState::Running {
            tracing::warn!("[sandbox:shell] a process is still running after its cancel");
        }
        Ok(read)
    }

    async fn read_until(&mut self, until: tokio::time::Instant) -> anyhow::Result<LiveRead> {
        use contract::{ExecutionState, OutputStream};
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let done = |stdout: &[u8], stderr: &[u8], state| LiveRead {
            stdout: String::from_utf8_lossy(stdout).into_owned(),
            stderr: String::from_utf8_lossy(stderr).into_owned(),
            state,
        };
        if self.released {
            return Ok(done(&stdout, &stderr, LiveState::Stopped));
        }
        loop {
            let batch = tinybox::read_output(&self.config, self.process.clone(), self.cursor)
                .await
                .map_err(|_| anyhow::anyhow!("TinyBox could not read command output"))?;
            let has_chunks = !batch.chunks.is_empty();
            for chunk in batch.chunks {
                match chunk.stream {
                    OutputStream::Stdout => stdout.extend_from_slice(&chunk.bytes),
                    OutputStream::Stderr => stderr.extend_from_slice(&chunk.bytes),
                }
            }
            self.cursor = batch.next_sequence;
            let state = match batch.state {
                ExecutionState::Running => {
                    if tokio::time::Instant::now() >= until {
                        return Ok(done(&stdout, &stderr, LiveState::Running));
                    }
                    tokio::time::sleep(READ_INTERVAL).await;
                    continue;
                }
                _ if has_chunks => continue,
                ExecutionState::Exited { exit_code } => LiveState::Exited(exit_code),
                ExecutionState::Cancelled => LiveState::Stopped,
                ExecutionState::Failed { code } => {
                    tracing::debug!(?code, "[sandbox:shell] a running process failed");
                    match super::shell_exec::failure_timed_out(code) {
                        Ok(true) => LiveState::TimedOut,
                        Ok(false) => LiveState::Stopped,
                        Err(error) => {
                            self.release().await;
                            return Err(error);
                        }
                    }
                }
            };
            self.release().await;
            return Ok(done(&stdout, &stderr, state));
        }
    }

    /// Release the stopped process's output and close its resource. Once.
    async fn release(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        let _ = tinybox::release_output(&self.config, self.process.clone()).await;
        if tinybox::close(&self.config, self.resource.clone())
            .await
            .is_err()
        {
            tracing::warn!("[sandbox:shell] TinyBox did not acknowledge a process cleanup");
        }
    }
}

#[cfg(feature = "modules")]
impl Drop for TinyBoxLive {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::warn!("[sandbox:shell] no runtime to stop a running process at turn end");
            return;
        };
        let (config, resource, process) = (
            self.config.clone(),
            self.resource.clone(),
            self.process.clone(),
        );
        tracing::debug!("[sandbox:shell] turn ended: stopping a process it left running");
        runtime.spawn(async move {
            let _ = tinybox::cancel(&config, process.clone()).await;
            let deadline = tokio::time::Instant::now() + KILL_GRACE;
            while tokio::time::Instant::now() < deadline {
                match tinybox::is_running(&config, process.clone()).await {
                    Ok(true) => tokio::time::sleep(Duration::from_millis(100)).await,
                    _ => break,
                }
            }
            let _ = tinybox::release_output(&config, process).await;
            let _ = tinybox::close(&config, resource).await;
        });
    }
}
