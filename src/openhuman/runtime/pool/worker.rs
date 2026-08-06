//! A single pooled worker: one long-lived interpreter child speaking the
//! newline-delimited JSON [`protocol`](super::protocol) over an isolated
//! loopback socket (with a stdio fallback for development harnesses).
//!
//! One worker runs **one job at a time**; concurrency comes from the
//! [`LangPool`](super::pool::LangPool) holding several workers. A worker stays
//! warm between jobs (the whole point — no per-run interpreter spawn) until it
//! is idle-reaped or recycled after N jobs.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, Lines};
use tokio::net::TcpListener;
use tokio::process::{Child, ChildStderr, ChildStdout, Command};

use super::protocol::{PoolJobRequest, PoolJobResponse, PoolReadyLine, PROTOCOL_VERSION};
use super::types::PoolLang;

/// How long to wait for a freshly-spawned worker to print its ready line.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

/// Everything needed to (re)spawn a worker for one language. Cheap to clone so
/// the pool can respawn on demand.
#[derive(Debug, Clone)]
pub struct WorkerLaunch {
    pub lang: PoolLang,
    /// Interpreter binary (`node` / `python`).
    pub bin: PathBuf,
    /// Args after the binary — typically `[harness_script_path]`.
    pub args: Vec<String>,
    /// Full environment for the child (already allow-listed by the backend).
    /// The child's env is cleared first, so this is the complete set.
    pub env: Vec<(String, String)>,
    /// Keep user fd 0/1/2 away from the NDJSON request/response stream by
    /// serving the protocol over a per-launch authenticated loopback socket.
    pub isolated_protocol: bool,
}

/// Failure from [`PoolWorker::submit`], tagged with whether the job was already
/// dispatched to the worker. A retry / legacy fallback is only safe when the job
/// was **not** dispatched (it never ran); a post-dispatch failure is terminal so
/// the same job is never executed twice.
#[derive(Debug)]
pub struct SubmitError {
    pub err: anyhow::Error,
    pub dispatched: bool,
}

impl SubmitError {
    fn pre(err: anyhow::Error) -> Self {
        Self {
            err,
            dispatched: false,
        }
    }
    fn post(err: anyhow::Error) -> Self {
        Self {
            err,
            dispatched: true,
        }
    }
}

impl std::fmt::Display for SubmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#}", self.err)
    }
}

/// A warm interpreter child plus its bookkeeping.
pub struct PoolWorker {
    launch: WorkerLaunch,
    _child: Child,
    stdin: Box<dyn AsyncWrite + Send + Unpin>,
    responses: Lines<BufReader<Box<dyn AsyncRead + Send + Unpin>>>,
    jobs_done: u64,
    last_used: Instant,
}

impl PoolWorker {
    pub fn jobs_done(&self) -> u64 {
        self.jobs_done
    }

    pub fn last_used(&self) -> Instant {
        self.last_used
    }

    /// Spawn a new worker and complete the readiness handshake.
    pub async fn spawn(launch: &WorkerLaunch) -> Result<Self> {
        tracing::info!(
            lang = launch.lang.id(),
            bin = %launch.bin.display(),
            "[runtime_pool] spawning worker"
        );
        let mut cmd = Command::new(&launch.bin);
        cmd.args(&launch.args);
        cmd.env_clear();
        for (key, value) in &launch.env {
            cmd.env(key, value);
        }
        let isolated_protocol = if launch.isolated_protocol {
            let listener = TcpListener::bind(("127.0.0.1", 0))
                .await
                .context("binding isolated worker protocol listener")?;
            let addr = listener
                .local_addr()
                .context("reading isolated worker protocol address")?;
            let token = uuid::Uuid::new_v4().to_string();
            cmd.env("OPENHUMAN_RUNTIME_POOL_PROTOCOL_ADDR", addr.to_string());
            cmd.env("OPENHUMAN_RUNTIME_POOL_PROTOCOL_TOKEN", &token);
            Some((listener, token))
        } else {
            None
        };
        if isolated_protocol.is_some() {
            // Jobs inherit EOF on fd 0, matching Command::output(), while the
            // harness receives requests over the isolated duplex socket.
            cmd.stdin(Stdio::null());
        } else {
            cmd.stdin(Stdio::piped());
        }
        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // Suppress the Windows console flash for each spawned worker.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawning {} worker", launch.lang.id()))?;
        let child_stdin = child.stdin.take();
        let stdout = child.stdout.take().context("worker stdout missing")?;
        if let Some(stderr) = child.stderr.take() {
            drain_stderr(launch.lang, stderr);
        }
        let (stdin, reader, expected_token): (
            Box<dyn AsyncWrite + Send + Unpin>,
            Box<dyn AsyncRead + Send + Unpin>,
            Option<String>,
        ) = if let Some((listener, token)) = isolated_protocol {
            // stdout is now exclusively user fd-level output. Drain it so
            // chatty jobs cannot block; protocol frames use the socket.
            drain_stdout(launch.lang, stdout);
            let (stream, _) = tokio::time::timeout(HANDSHAKE_TIMEOUT, listener.accept())
                .await
                .map_err(|_| {
                    anyhow::anyhow!("{} worker protocol connection timed out", launch.lang.id())
                })?
                .context("accepting isolated worker protocol connection")?;
            let (reader, writer) = tokio::io::split(stream);
            (Box::new(writer), Box::new(reader), Some(token))
        } else {
            (
                Box::new(child_stdin.context("worker stdin missing")?),
                Box::new(stdout),
                None,
            )
        };
        let mut lines = BufReader::new(reader).lines();

        let ready_line = match tokio::time::timeout(HANDSHAKE_TIMEOUT, lines.next_line()).await {
            Ok(Ok(Some(line))) => line,
            Ok(Ok(None)) => bail!(
                "{} worker exited before readiness handshake",
                launch.lang.id()
            ),
            Ok(Err(error)) => {
                return Err(error).context("reading worker handshake");
            }
            Err(_) => bail!("{} worker readiness handshake timed out", launch.lang.id()),
        };
        let ready: PoolReadyLine = serde_json::from_str(&ready_line)
            .with_context(|| format!("parsing worker ready line: {ready_line}"))?;
        if !ready.ready {
            bail!(
                "{} worker failed to start: {}",
                launch.lang.id(),
                ready.error.unwrap_or_else(|| "unknown".to_string())
            );
        }
        if ready.protocol != Some(PROTOCOL_VERSION) {
            bail!(
                "{} worker protocol mismatch: expected {}, got {:?}",
                launch.lang.id(),
                PROTOCOL_VERSION,
                ready.protocol
            );
        }
        if ready.protocol_token != expected_token {
            bail!("{} worker protocol authentication failed", launch.lang.id());
        }
        tracing::info!(lang = launch.lang.id(), "[runtime_pool] worker ready");

        Ok(Self {
            launch: launch.clone(),
            _child: child,
            stdin,
            responses: lines,
            jobs_done: 0,
            last_used: Instant::now(),
        })
    }

    /// Submit one job and await its response.
    ///
    /// `hard_timeout` is a **safety net** above the worker's own soft deadline:
    /// the worker aborts a job at `req.timeout_ms` and still replies, so this
    /// only fires if the worker itself has wedged. On `Err` the caller must
    /// discard this worker — its stdio framing can no longer be trusted.
    pub async fn submit(
        &mut self,
        req: &PoolJobRequest,
        hard_timeout: Option<Duration>,
    ) -> std::result::Result<PoolJobResponse, SubmitError> {
        let mut line = serde_json::to_string(req)
            .map_err(|e| SubmitError::pre(anyhow::Error::new(e).context("serialising pool job")))?;
        line.push('\n');
        // A write failure means the bytes never reached the worker (e.g. a
        // reused idle worker died) → the job did not run → safe to retry.
        self.stdin.write_all(line.as_bytes()).await.map_err(|e| {
            SubmitError::pre(anyhow::Error::new(e).context("writing pool job request"))
        })?;
        // Past this point the request bytes are in the pipe: the job may execute,
        // so any later failure is terminal (never re-run the same job).
        self.stdin.flush().await.map_err(|e| {
            SubmitError::post(anyhow::Error::new(e).context("flushing pool job request"))
        })?;

        // Fixed deadline: `continue`ing over unparseable / mismatched-id lines
        // must NOT reset the wedged-worker timeout, so it bounds the total wait.
        let deadline = hard_timeout.map(|t| tokio::time::Instant::now() + t);
        loop {
            let next = match deadline {
                Some(dl) => match tokio::time::timeout_at(dl, self.responses.next_line()).await {
                    Ok(inner) => inner,
                    Err(_) => {
                        return Err(SubmitError::post(anyhow::anyhow!(
                            "pool worker job timed out (hard deadline; worker wedged)"
                        )))
                    }
                },
                None => self.responses.next_line().await,
            };
            let line = match next {
                Ok(Some(line)) => line,
                Ok(None) => {
                    return Err(SubmitError::post(anyhow::anyhow!(
                        "pool worker closed stdout"
                    )))
                }
                Err(error) => {
                    return Err(SubmitError::post(
                        anyhow::Error::new(error).context("reading pool job response"),
                    ))
                }
            };
            let response: PoolJobResponse = match serde_json::from_str(&line) {
                Ok(response) => response,
                Err(error) => {
                    tracing::warn!(
                        lang = self.launch.lang.id(),
                        "[runtime_pool] unparseable worker line skipped: {error}"
                    );
                    continue;
                }
            };
            if response.id.as_deref() != Some(req.id.as_str()) {
                tracing::debug!(
                    lang = self.launch.lang.id(),
                    "[runtime_pool] skipped response for different id={:?}",
                    response.id
                );
                continue;
            }
            self.jobs_done += 1;
            self.last_used = Instant::now();
            return Ok(response);
        }
    }

    /// Whether this worker has served enough jobs to be recycled. `0` disables.
    pub fn should_recycle(&self, recycle_after: u64) -> bool {
        recycle_due(self.jobs_done, recycle_after)
    }

    /// Whether this worker has been idle at least `ttl`.
    pub fn idle_expired(&self, ttl: Duration) -> bool {
        idle_due(self.last_used.elapsed(), ttl)
    }

    /// Signal the child to exit. Best-effort; `kill_on_drop` is the backstop.
    pub fn shutdown(mut self) {
        if let Err(error) = self._child.start_kill() {
            tracing::debug!(
                lang = self.launch.lang.id(),
                "[runtime_pool] failed to signal worker shutdown: {error}"
            );
        }
    }
}

/// Continuously drain a worker's stderr so a chatty child never blocks on a
/// full pipe. Lines are logged at trace; never parsed as protocol.
fn drain_stderr(lang: PoolLang, stderr: ChildStderr) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            tracing::trace!(lang = lang.id(), "[runtime_pool] worker stderr: {line}");
        }
    });
}

/// Drain fd-level stdout from workers whose protocol uses an isolated socket.
/// This output is deliberately never parsed as NDJSON, so user code cannot
/// forge a response frame or desynchronise subsequent jobs.
fn drain_stdout(lang: PoolLang, stdout: ChildStdout) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            tracing::trace!(lang = lang.id(), "[runtime_pool] worker fd stdout: {line}");
        }
    });
}

/// Pure recycle predicate: a worker is due for recycling once it has served
/// `recycle_after` jobs (`0` disables recycling).
fn recycle_due(jobs_done: u64, recycle_after: u64) -> bool {
    recycle_after > 0 && jobs_done >= recycle_after
}

/// Pure idle-expiry predicate: idle for at least `ttl`.
fn idle_due(idle_elapsed: Duration, ttl: Duration) -> bool {
    idle_elapsed >= ttl
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recycle_due_respects_budget_and_disable() {
        assert!(!recycle_due(0, 0), "recycle_after=0 disables recycling");
        assert!(!recycle_due(100, 0), "recycle_after=0 never recycles");
        assert!(!recycle_due(4, 5), "below budget");
        assert!(recycle_due(5, 5), "at budget");
        assert!(recycle_due(6, 5), "past budget");
    }

    #[test]
    fn idle_due_is_inclusive_at_ttl() {
        assert!(!idle_due(Duration::from_secs(4), Duration::from_secs(5)));
        assert!(idle_due(Duration::from_secs(5), Duration::from_secs(5)));
        assert!(idle_due(Duration::from_secs(6), Duration::from_secs(5)));
    }

    #[test]
    fn submit_error_tags_dispatch_state() {
        let pre = SubmitError::pre(anyhow::anyhow!("write failed"));
        assert!(
            !pre.dispatched,
            "write failures are pre-dispatch (retryable)"
        );
        let post = SubmitError::post(anyhow::anyhow!("read timed out"));
        assert!(
            post.dispatched,
            "read failures are post-dispatch (terminal)"
        );
    }
}
