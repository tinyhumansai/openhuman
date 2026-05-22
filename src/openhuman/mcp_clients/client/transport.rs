//! Low-level stdio transport for MCP JSON-RPC.
//!
//! Owns the child process lifecycle, the reader task that deserializes
//! newline-delimited JSON from stdout, and the write-half that serializes
//! requests to stdin. A ring buffer captures stderr lines for error reporting.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::{oneshot, Mutex, RwLock};

pub type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

/// Capacity of the stderr ring buffer (lines).
const STDERR_RING_SIZE: usize = 64;

/// MCP protocol version negotiated in `initialize`.
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Serialised, newline-terminated JSON to be written to stdin.
pub struct TransportWriter {
    stdin: ChildStdin,
}

impl TransportWriter {
    pub fn new(stdin: ChildStdin) -> Self {
        Self { stdin }
    }

    /// Write a single JSON value followed by `\n` and flush.
    pub async fn send(&mut self, msg: &Value) -> anyhow::Result<()> {
        let mut bytes = serde_json::to_vec(msg)?;
        bytes.push(b'\n');
        self.stdin.write_all(&bytes).await?;
        self.stdin.flush().await?;
        Ok(())
    }
}

/// Owns the stdout reader task; routes responses to waiting callers.
pub struct TransportReader {
    pub pending: PendingMap,
    pub stderr_ring: Arc<RwLock<VecDeque<String>>>,
}

impl TransportReader {
    pub fn new() -> Self {
        Self {
            pending: Arc::new(Mutex::new(HashMap::new())),
            stderr_ring: Arc::new(RwLock::new(VecDeque::with_capacity(STDERR_RING_SIZE))),
        }
    }

    /// Spawn a background task that drains `stdout` and resolves waiters.
    ///
    /// When the reader exits (EOF or error), all pending waiters are flushed
    /// with an error so they do not leak and wait until their own timeout fires.
    pub fn spawn_reader(&self, stdout: ChildStdout, server_id: String) {
        let pending = Arc::clone(&self.pending);
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                // Do NOT log raw payload — MCP subprocesses may emit secrets or PII.
                tracing::trace!(
                    "[mcp-client] server_id={} received stdout line (len={})",
                    server_id,
                    line.len()
                );
                let parsed: Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(
                            "[mcp-client] server_id={} unparseable stdout line: {e}",
                            server_id
                        );
                        continue;
                    }
                };

                // Route responses to waiting callers by id
                if let Some(id) = parsed.get("id").and_then(Value::as_u64) {
                    let mut map = pending.lock().await;
                    if let Some(tx) = map.remove(&id) {
                        let result = if let Some(err) = parsed.get("error") {
                            Err(err.to_string())
                        } else {
                            Ok(parsed.get("result").cloned().unwrap_or(Value::Null))
                        };
                        let _ = tx.send(result);
                    }
                }
                // Notifications have no id — log and ignore
            }
            tracing::debug!(
                "[mcp-client] stdout reader exiting for server_id={}",
                server_id
            );
            // Flush all pending waiters with an error so they don't leak until timeout.
            let mut map = pending.lock().await;
            for (id, tx) in map.drain() {
                tracing::debug!(
                    "[mcp-client] server_id={} flushing pending waiter id={}",
                    server_id,
                    id
                );
                let _ = tx.send(Err("MCP server stdout closed unexpectedly".to_string()));
            }
        });
    }

    /// Spawn a background task that drains `stderr` into the ring buffer.
    ///
    /// Raw stderr content is NOT logged — MCP subprocesses may emit secrets or
    /// PII. Only the line length is traced for diagnostics.
    pub fn spawn_stderr_reader(&self, stderr: tokio::process::ChildStderr, server_id: String) {
        let ring = Arc::clone(&self.stderr_ring);
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::trace!(
                    "[mcp-client] server_id={} received stderr line (len={})",
                    server_id,
                    line.len()
                );
                let mut buf = ring.write().await;
                if buf.len() >= STDERR_RING_SIZE {
                    buf.pop_front();
                }
                buf.push_back(line);
            }
        });
    }

    /// Return the most recent stderr line, if any.
    pub async fn last_stderr(&self) -> Option<String> {
        self.stderr_ring.read().await.back().cloned()
    }
}

/// Wrap up a just-spawned child and its I/O halves.
pub struct SpawnedProcess {
    pub child: Child,
    pub writer: TransportWriter,
    pub reader: TransportReader,
}

impl SpawnedProcess {
    pub fn from_child(mut child: Child, server_id: &str) -> anyhow::Result<Self> {
        let stdin = child.stdin.take().ok_or_else(|| {
            anyhow::anyhow!("[mcp-client] server_id={server_id} failed to take stdin")
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            anyhow::anyhow!("[mcp-client] server_id={server_id} failed to take stdout")
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            anyhow::anyhow!("[mcp-client] server_id={server_id} failed to take stderr")
        })?;

        let writer = TransportWriter::new(stdin);
        let reader = TransportReader::new();
        reader.spawn_reader(stdout, server_id.to_string());
        reader.spawn_stderr_reader(stderr, server_id.to_string());

        Ok(Self {
            child,
            writer,
            reader,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mcp_protocol_version_is_2024_11_05() {
        assert_eq!(MCP_PROTOCOL_VERSION, "2024-11-05");
    }

    #[tokio::test]
    async fn pending_map_insert_and_remove() {
        let map: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = oneshot::channel::<Result<Value, String>>();
        {
            let mut guard = map.lock().await;
            guard.insert(42, tx);
            assert_eq!(guard.len(), 1);
        }
        {
            let mut guard = map.lock().await;
            let sender = guard.remove(&42).unwrap();
            sender.send(Ok(json!("ok"))).unwrap();
        }
        assert_eq!(rx.await.unwrap().unwrap(), json!("ok"));
    }

    #[tokio::test]
    async fn stderr_ring_caps_at_max_size() {
        let ring: Arc<RwLock<VecDeque<String>>> =
            Arc::new(RwLock::new(VecDeque::with_capacity(STDERR_RING_SIZE)));
        for i in 0..(STDERR_RING_SIZE + 10) {
            let mut buf = ring.write().await;
            if buf.len() >= STDERR_RING_SIZE {
                buf.pop_front();
            }
            buf.push_back(format!("line {i}"));
        }
        let buf = ring.read().await;
        assert_eq!(buf.len(), STDERR_RING_SIZE);
        // The oldest (line 0 .. 9) should have been evicted
        assert!(buf.front().unwrap().starts_with("line 10"));
    }
}
