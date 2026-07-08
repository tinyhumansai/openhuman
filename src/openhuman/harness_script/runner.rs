//! Phase 1 runner skeleton for LLM-authored Python harness scripts.
//!
//! [`PythonHarnessScriptRunner`] resolves a managed interpreter through
//! [`crate::openhuman::runtime_python::PythonBootstrap`], launches the embedded
//! [`bootstrap.py`](super) child, drives the newline-JSON handshake, and runs a
//! single script to a terminal result/error under wall-time and output-byte
//! caps with cooperative cancellation. The child is always killed and reaped
//! before `run` returns.
//!
//! Phase 1 wires no orchestration bridge, tool surface, or sandbox — those are
//! Phases 2-4. See `docs/superpowers/specs/2026-07-09-harness-script-runner-design.md`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdout};
use tokio_util::sync::CancellationToken;

use crate::openhuman::runtime_python::process::PythonLaunchSpec;
use crate::openhuman::runtime_python::PythonBootstrap;

use super::protocol::{
    decode_line, encode_line, ChildMessage, InitMessage, ScriptLimits, PROTOCOL_VERSION,
};

/// Default embedded child bootstrap. Overridable per-runner via
/// [`PythonHarnessScriptRunner::with_bootstrap_source`] for tests.
const DEFAULT_BOOTSTRAP: &str = include_str!("bootstrap.py");

/// How long to wait for the child's `ready` handshake before giving up.
const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

/// What the caller wants to run.
#[derive(Debug, Clone)]
pub struct ScriptRunSpec {
    /// Python source to execute in the child.
    pub script: String,
    /// Arbitrary JSON exposed to the script as `inputs`.
    pub inputs: Value,
    /// Wall-clock budget for the script to reach a terminal message.
    pub wall_time: Duration,
    /// Cap on buffered protocol stdout before aborting.
    pub max_stdout_bytes: usize,
    /// Cap on retained stderr log bytes.
    pub max_stderr_bytes: usize,
}

impl ScriptRunSpec {
    /// A spec with conservative Phase 1 defaults; caps chosen to keep a runaway
    /// child from exhausting memory while leaving room for real output.
    pub fn new(script: impl Into<String>, inputs: Value) -> Self {
        Self {
            script: script.into(),
            inputs,
            wall_time: Duration::from_secs(60),
            max_stdout_bytes: 4 * 1024 * 1024,
            max_stderr_bytes: 256 * 1024,
        }
    }
}

/// Successful terminal state of a script run.
#[derive(Debug, Clone)]
pub struct ScriptRunOutcome {
    /// Value the script passed to `set_result(...)` (JSON `null` if unset).
    pub output: Value,
    /// Captured child stderr, truncated to the configured cap.
    pub stderr_log: String,
    /// Whether stderr was truncated at the cap.
    pub stderr_truncated: bool,
}

/// Failure modes of a script run. Variants are deliberately specific so a
/// future LLM caller (Phase 3) can distinguish "fix your script" from
/// "the runtime gave up" and repair accordingly.
#[derive(Debug, thiserror::Error)]
pub enum HarnessScriptError {
    #[error("failed to launch python harness-script child: {0:#}")]
    Spawn(#[source] anyhow::Error),

    #[error("child did not complete the handshake within {0:?}")]
    HandshakeTimeout(Duration),

    #[error("protocol version mismatch: runner speaks {expected}, child announced {actual}")]
    ProtocolMismatch { expected: u32, actual: u32 },

    #[error("malformed protocol message from child: {0}")]
    MalformedMessage(String),

    #[error("script exceeded its wall-time budget of {0:?}")]
    WallTimeout(Duration),

    #[error("script run was cancelled")]
    Cancelled,

    #[error("script raised {error_class}: {message}")]
    ScriptError {
        error_class: String,
        message: String,
        traceback: Option<String>,
    },

    #[error("child protocol stdout exceeded the {0}-byte cap")]
    OutputCapExceeded(usize),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Launches and supervises Python harness-script children.
pub struct PythonHarnessScriptRunner {
    bootstrap: Arc<PythonBootstrap>,
    bootstrap_source: String,
    handshake_timeout: Duration,
}

impl PythonHarnessScriptRunner {
    /// Construct a runner over an already-configured [`PythonBootstrap`].
    pub fn new(bootstrap: Arc<PythonBootstrap>) -> Self {
        Self {
            bootstrap,
            bootstrap_source: DEFAULT_BOOTSTRAP.to_string(),
            handshake_timeout: DEFAULT_HANDSHAKE_TIMEOUT,
        }
    }

    /// Override the child bootstrap source. Test seam: lets tests inject a
    /// pathological child (bad handshake, sleeper, flooder) to exercise the
    /// runner's failure paths without a real orchestration layer.
    pub fn with_bootstrap_source(mut self, source: impl Into<String>) -> Self {
        self.bootstrap_source = source.into();
        self
    }

    /// Override the handshake timeout (mainly for tests).
    pub fn with_handshake_timeout(mut self, timeout: Duration) -> Self {
        self.handshake_timeout = timeout;
        self
    }

    /// Run one script to completion. Always kills and reaps the child before
    /// returning, whether it succeeded, timed out, or was cancelled.
    pub async fn run(
        &self,
        spec: ScriptRunSpec,
        cancel: CancellationToken,
    ) -> Result<ScriptRunOutcome, HarnessScriptError> {
        let script_path = self
            .materialize_bootstrap()
            .await
            .map_err(HarnessScriptError::Spawn)?;

        let launch = PythonLaunchSpec::new(script_path.clone());
        let spawned = self
            .bootstrap
            .spawn_stdio(&launch)
            .await
            .map_err(HarnessScriptError::Spawn);
        let mut child = match spawned {
            Ok(child) => child,
            Err(e) => {
                // Nothing launched; drop the temp script and bail.
                let _ = tokio::fs::remove_file(&script_path).await;
                return Err(e);
            }
        };

        let result = self.drive(&mut child, &spec, &cancel).await;
        cleanup_child(&mut child).await;
        // Child is dead and reaped; the interpreter no longer needs the source.
        let _ = tokio::fs::remove_file(&script_path).await;
        result
    }

    /// Write the (possibly test-injected) bootstrap source to a unique file in
    /// the harness-script cache dir. A per-run uuid name avoids races between
    /// concurrent runs that use different sources.
    async fn materialize_bootstrap(&self) -> anyhow::Result<PathBuf> {
        let dir = harness_script_cache_dir();
        tokio::fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("creating harness-script cache dir {}", dir.display()))?;
        let path = dir.join(format!("bootstrap-{}.py", uuid::Uuid::new_v4()));
        tokio::fs::write(&path, self.bootstrap_source.as_bytes())
            .await
            .with_context(|| format!("writing harness-script bootstrap {}", path.display()))?;
        Ok(path)
    }

    /// Handshake → send init → await terminal, racing wall-time and cancel.
    async fn drive(
        &self,
        child: &mut Child,
        spec: &ScriptRunSpec,
        cancel: &CancellationToken,
    ) -> Result<ScriptRunOutcome, HarnessScriptError> {
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or_else(|| {
            HarnessScriptError::MalformedMessage("child stdout unavailable".into())
        })?;
        let stderr = child.stderr.take();

        // Drain stderr concurrently so a chatty child can't deadlock on a full
        // pipe while we wait on stdout.
        let max_stderr = spec.max_stderr_bytes;
        let stderr_task = tokio::spawn(async move {
            match stderr {
                Some(s) => drain_capped(s, max_stderr).await,
                None => (String::new(), false),
            }
        });

        let mut lines = BufReader::new(stdout).lines();

        let handshake = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(HarnessScriptError::Cancelled),
            res = tokio::time::timeout(self.handshake_timeout, lines.next_line()) => match res {
                Err(_) => Err(HarnessScriptError::HandshakeTimeout(self.handshake_timeout)),
                Ok(Ok(Some(line))) => Ok(line),
                Ok(Ok(None)) => Err(HarnessScriptError::MalformedMessage(
                    "child closed stdout before handshake".into(),
                )),
                Ok(Err(e)) => Err(HarnessScriptError::Io(e)),
            }
        };
        let handshake = handshake?;
        parse_handshake(&handshake)?;

        // Handshake good — send init.
        let init = InitMessage::new(
            spec.script.clone(),
            spec.inputs.clone(),
            ScriptLimits {
                wall_time_ms: spec.wall_time.as_millis() as u64,
                max_stdout_bytes: spec.max_stdout_bytes,
                max_stderr_bytes: spec.max_stderr_bytes,
            },
        );
        let line = encode_line(&init)
            .map_err(|e| HarnessScriptError::MalformedMessage(format!("encoding init: {e}")))?;
        if let Some(mut stdin) = stdin {
            stdin.write_all(line.as_bytes()).await?;
            stdin.flush().await?;
            // Drop stdin to signal EOF; the Phase 1 child reads only `init`.
            drop(stdin);
        }

        // Await the terminal message, racing the wall-clock budget and cancel.
        let terminal = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(HarnessScriptError::Cancelled),
            res = tokio::time::timeout(
                spec.wall_time,
                read_terminal(&mut lines, spec.max_stdout_bytes),
            ) => match res {
                Err(_) => Err(HarnessScriptError::WallTimeout(spec.wall_time)),
                Ok(inner) => inner,
            }
        };

        let (stderr_log, stderr_truncated) = collect_stderr(stderr_task).await;

        match terminal? {
            ChildMessage::Result { output } => Ok(ScriptRunOutcome {
                output,
                stderr_log,
                stderr_truncated,
            }),
            ChildMessage::Error {
                error_class,
                message,
                traceback,
            } => Err(HarnessScriptError::ScriptError {
                error_class,
                message,
                traceback,
            }),
            ChildMessage::Ready { .. } => Err(HarnessScriptError::MalformedMessage(
                "child sent a second `ready` instead of a terminal message".into(),
            )),
            ChildMessage::Unknown => Err(HarnessScriptError::MalformedMessage(
                "child sent an unrecognized frame before any terminal message".into(),
            )),
        }
    }
}

/// Validate the first stdout line as a version-matched `ready` frame.
fn parse_handshake(line: &str) -> Result<(), HarnessScriptError> {
    match decode_line(line)
        .map_err(|e| HarnessScriptError::MalformedMessage(format!("handshake: {e}")))?
    {
        ChildMessage::Ready { protocol_version } if protocol_version == PROTOCOL_VERSION => Ok(()),
        ChildMessage::Ready { protocol_version } => Err(HarnessScriptError::ProtocolMismatch {
            expected: PROTOCOL_VERSION,
            actual: protocol_version,
        }),
        other => Err(HarnessScriptError::MalformedMessage(format!(
            "expected `ready` handshake, got {other:?}"
        ))),
    }
}

/// Read the single post-handshake frame the child is expected to send,
/// enforcing the stdout byte cap. Phase 1 has no intermediate frames; the
/// caller treats a non-terminal frame here as a protocol error. (Phase 2 will
/// grow this into a loop that services `call` request frames before the
/// terminal message.)
async fn read_terminal(
    lines: &mut Lines<BufReader<ChildStdout>>,
    max_stdout_bytes: usize,
) -> Result<ChildMessage, HarnessScriptError> {
    match lines.next_line().await? {
        None => Err(HarnessScriptError::MalformedMessage(
            "child exited before sending a terminal message".into(),
        )),
        Some(line) => {
            if line.len() + 1 > max_stdout_bytes {
                return Err(HarnessScriptError::OutputCapExceeded(max_stdout_bytes));
            }
            decode_line(&line)
                .map_err(|e| HarnessScriptError::MalformedMessage(format!("terminal read: {e}")))
        }
    }
}

/// Read a child pipe to EOF, keeping at most `cap` bytes. Returns the captured
/// text and whether it was truncated.
async fn drain_capped<R>(mut reader: R, cap: usize) -> (String, bool)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut truncated = false;
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if buf.len() < cap {
                    let room = cap - buf.len();
                    let take = room.min(n);
                    buf.extend_from_slice(&chunk[..take]);
                    if take < n {
                        truncated = true;
                    }
                } else {
                    truncated = true;
                }
            }
        }
    }
    (String::from_utf8_lossy(&buf).into_owned(), truncated)
}

/// Await the stderr drain task, tolerating a panicked/cancelled join.
async fn collect_stderr(task: tokio::task::JoinHandle<(String, bool)>) -> (String, bool) {
    match tokio::time::timeout(Duration::from_secs(2), task).await {
        Ok(Ok(pair)) => pair,
        _ => (String::new(), false),
    }
}

/// Ensure the child is dead and reaped. `kill_on_drop` is the backstop, but we
/// reap explicitly so no zombie lingers between drop and the next poll.
async fn cleanup_child(child: &mut Child) {
    let _ = child.start_kill();
    let _ = child.wait().await;
}

/// Cache directory for materialized bootstrap scripts, mirroring how
/// `runtime_python` derives its cache root.
fn harness_script_cache_dir() -> PathBuf {
    if let Some(user_cache) = dirs::cache_dir() {
        return user_cache.join("openhuman").join("harness-script");
    }
    PathBuf::from(".openhuman").join("harness-script")
}
