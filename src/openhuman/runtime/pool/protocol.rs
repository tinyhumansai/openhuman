//! Wire protocol between the core and a pooled language worker.
//!
//! Newline-delimited JSON over a per-worker duplex transport, mirroring the
//! [`runtime_python_server`](crate::openhuman::runtime::python_server) protocol.
//! Production workers use an authenticated loopback socket so job fd 0/1/2
//! cannot consume or corrupt protocol traffic:
//!
//! 1. On startup the worker prints exactly one [`PoolReadyLine`].
//! 2. The core writes one [`PoolJobRequest`] per line; the worker replies with
//!    one [`PoolJobResponse`] per line, correlated by `id`.
//!
//! Child stdout/stderr are drained separately and never carry protocol frames.

use serde::{Deserialize, Serialize};

/// Bumped whenever the request/response shape changes incompatibly. The worker
/// echoes the version it speaks in its ready line; a mismatch fails the launch.
pub const PROTOCOL_VERSION: u32 = 1;

/// Handshake line printed once by the worker on startup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolReadyLine {
    #[serde(default)]
    pub ready: bool,
    #[serde(default)]
    pub protocol: Option<u32>,
    /// `"node"` / `"python"` — a sanity check that the right harness launched.
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    /// Optional per-launch secret used when the protocol travels over an
    /// isolated loopback socket instead of stdout.
    #[serde(default)]
    pub protocol_token: Option<String>,
}

/// A single unit of work sent to a worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolJobRequest {
    /// Correlation id — the worker echoes it back in the response.
    pub id: String,
    /// Job kind. Today only `"inline"` (evaluate `code`); reserved for future
    /// `"script"` support.
    pub kind: String,
    /// Inline source to evaluate (for `kind == "inline"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Working directory for the job. The worker `chdir`s per job so relative
    /// paths resolve against the caller's action sandbox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Soft per-job deadline in milliseconds. The worker aborts the job when it
    /// elapses and replies with `timed_out = true`. Absent ⇒ run to completion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

/// A worker's reply to a [`PoolJobRequest`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolJobResponse {
    pub id: Option<String>,
    /// `true` when the harness ran the job to a normal conclusion (the user
    /// code may still have thrown — see `exit_code`/`stderr`). `false` only for
    /// harness-level failures described in `error`.
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
    /// `0` on clean completion, non-zero when the job threw/exited non-zero,
    /// `None` when not applicable.
    #[serde(default)]
    pub exit_code: Option<i32>,
    /// Set when the worker aborted the job at its soft deadline.
    #[serde(default)]
    pub timed_out: bool,
    #[serde(default)]
    pub elapsed_ms: u64,
    /// Harness-level error (worker could not run the job at all).
    #[serde(default)]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_line_parses() {
        let ready: PoolReadyLine =
            serde_json::from_str(r#"{"ready":true,"protocol":1,"lang":"node"}"#).unwrap();
        assert!(ready.ready);
        assert_eq!(ready.protocol, Some(PROTOCOL_VERSION));
        assert_eq!(ready.lang.as_deref(), Some("node"));
    }

    #[test]
    fn request_omits_absent_optional_fields() {
        let req = PoolJobRequest {
            id: "3".to_string(),
            kind: "inline".to_string(),
            code: Some("console.log(1)".to_string()),
            cwd: None,
            timeout_ms: None,
        };
        let line = serde_json::to_string(&req).unwrap();
        assert!(line.contains("\"kind\":\"inline\""));
        assert!(!line.contains("cwd"));
        assert!(!line.contains("timeout_ms"));
    }

    #[test]
    fn response_parses_failure_envelope() {
        let resp: PoolJobResponse = serde_json::from_str(
            r#"{"id":"7","ok":true,"stdout":"","stderr":"boom","exit_code":1,"elapsed_ms":12}"#,
        )
        .unwrap();
        assert!(resp.ok);
        assert_eq!(resp.exit_code, Some(1));
        assert_eq!(resp.stderr, "boom");
        assert!(!resp.timed_out);
    }
}
