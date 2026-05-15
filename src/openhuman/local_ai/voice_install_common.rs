//! Shared installer plumbing for the local voice stack (Whisper + Piper).
//!
//! Both installers need the same primitives:
//!
//! - Stream a URL to disk via `.part` suffix + atomic rename so a crash
//!   never leaves a corrupt artifact that downstream code (the STT/TTS
//!   factory) tries to load.
//! - Validate either a known SHA256 (when upstream publishes one) or a
//!   minimum size threshold so a truncated download doesn't masquerade as
//!   a finished install.
//! - Surface per-engine progress (downloading, extracting, idle, ready,
//!   error) on a polled status RPC — matches the existing
//!   `local_ai_downloads_progress` UX so the VoicePanel can reuse the
//!   same progress UI primitives without inventing a new event-bus channel.
//!
//! The Ollama installer fires-and-forgets a single PowerShell / sh block
//! and lets the OS owner that process. For Whisper and Piper we need
//! finer-grained progress reporting (the GGML model file alone is up to
//! 1.6 GB and users absolutely will need a percentage indicator) so the
//! shared harness here streams the body chunks itself and updates a
//! singleton state map keyed by engine id.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Overall request timeout for a single install download. 30 minutes covers
/// the 1.6 GB `ggml-large-v3-turbo` model on a 1 Mbps link with headroom;
/// anything slower probably isn't realistically going to finish anyway.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(1800);

/// Per-chunk idle timeout. If the body stream produces no bytes for this
/// long, treat the connection as dead and abort so the caller can retry
/// from a clean state. Without this guard, a half-open TCP connection (the
/// failure mode behind the "progress stuck at 18%" symptom) holds the
/// install task forever, defeating the polled-status UX.
const CHUNK_IDLE_TIMEOUT: Duration = Duration::from_secs(45);
use tokio::io::AsyncWriteExt;

/// Stable engine id for status tracking. The two installers register their
/// progress under these keys; the status RPC reads them back.
pub const ENGINE_WHISPER: &str = "whisper";
pub const ENGINE_PIPER: &str = "piper";

/// Lifecycle state for a voice-engine install. Mirrors the state machine
/// the Ollama installer exposes via `LocalAiStatus.state`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VoiceInstallState {
    /// Nothing has happened — no binaries, no models. The default state
    /// when the workspace has never been touched.
    Missing,
    /// An install is in flight. `progress` and `downloaded_bytes` will be
    /// updated as chunks land.
    Installing,
    /// All required artifacts (binary + at least one default model) are
    /// present and pass validation.
    Installed,
    /// The expected install dir contains artifacts but they fail
    /// validation (e.g. size below threshold, hash mismatch, missing
    /// `.onnx.json` sidecar). The user should re-run install.
    Broken,
    /// The last install attempt errored. `error_detail` carries the
    /// human-readable reason.
    Error,
}

impl VoiceInstallState {
    pub fn as_str(&self) -> &'static str {
        match self {
            VoiceInstallState::Missing => "missing",
            VoiceInstallState::Installing => "installing",
            VoiceInstallState::Installed => "installed",
            VoiceInstallState::Broken => "broken",
            VoiceInstallState::Error => "error",
        }
    }
}

/// Snapshot returned over JSON-RPC for one engine's installer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceInstallStatus {
    /// Stable engine id (`"whisper"` / `"piper"`).
    pub engine: String,
    /// Current state — see [`VoiceInstallState`].
    pub state: VoiceInstallState,
    /// 0-100 percent for the in-flight download (`None` when state is not
    /// `Installing`).
    pub progress: Option<u8>,
    /// Bytes received so far.
    pub downloaded_bytes: Option<u64>,
    /// Total bytes expected (from `Content-Length` — may be `None` for
    /// chunked transfer encoding).
    pub total_bytes: Option<u64>,
    /// Free-text status line — what file we're downloading, what stage
    /// we're at. Useful for the UI to show "Downloading whisper-cli…" vs
    /// "Downloading ggml-large-v3-turbo.bin…".
    pub stage: Option<String>,
    /// Populated when `state == Error` — the user-facing failure reason.
    pub error_detail: Option<String>,
}

impl VoiceInstallStatus {
    fn missing(engine: &str) -> Self {
        Self {
            engine: engine.to_string(),
            state: VoiceInstallState::Missing,
            progress: None,
            downloaded_bytes: None,
            total_bytes: None,
            stage: None,
            error_detail: None,
        }
    }
}

/// In-memory status table — keyed by engine id. Both installers share
/// this so the status RPC can answer for either engine without a separate
/// store.
static STATUS_TABLE: once_cell::sync::Lazy<Mutex<HashMap<String, VoiceInstallStatus>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(HashMap::new()));

/// Fetch the current status snapshot for `engine`. Returns `Missing`
/// when the engine has never been touched.
pub fn read_status(engine: &str) -> VoiceInstallStatus {
    STATUS_TABLE
        .lock()
        .expect("voice install status lock poisoned")
        .get(engine)
        .cloned()
        .unwrap_or_else(|| VoiceInstallStatus::missing(engine))
}

/// Replace the snapshot for `engine`. Internal helper for the installer
/// flow — exposed at module scope so install_whisper / install_piper can
/// update progress without going through a public setter API.
pub(crate) fn write_status(status: VoiceInstallStatus) {
    log::debug!(
        "[voice-install] status update engine={} state={} progress={:?} stage={:?}",
        status.engine,
        status.state.as_str(),
        status.progress,
        status.stage,
    );
    let mut table = STATUS_TABLE
        .lock()
        .expect("voice install status lock poisoned");
    table.insert(status.engine.clone(), status);
}

/// Force a fresh missing state for `engine`. Used by tests and by the
/// "Reinstall" path before kicking off a new download.
#[cfg(test)]
pub(crate) fn reset_status(engine: &str) {
    let mut table = STATUS_TABLE
        .lock()
        .expect("voice install status lock poisoned");
    table.remove(engine);
}

/// Download `url` to `dest` with atomic rename. Streams bytes through
/// SHA256 if `expected_sha256` is provided, otherwise validates that the
/// final size is at least `min_bytes`.
///
/// The on-disk write goes to `<dest>.part` first and is `rename`d into
/// place only after all checks pass. If the function is interrupted
/// mid-stream (process killed, network drop) the `.part` file is the
/// only thing left behind; the next call detects and overwrites it so
/// we never read a half-written model.
///
/// Progress callbacks fire every chunk with `(downloaded_bytes,
/// total_bytes)`. Total may be `None` for chunked responses.
pub async fn download_to_file(
    url: &str,
    dest: &Path,
    expected_sha256: Option<&str>,
    min_bytes: u64,
    log_prefix: &str,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("{log_prefix} mkdir {}: {e}", parent.display()))?;
    }

    let part_path = part_path(dest);
    // Always start from scratch — resumable HTTP Range support is
    // useful but not free (servers must return 206, hash state has to
    // restart on hash mismatch). For the MVP we restart cleanly and
    // ensure `.part` is removed first so we never accidentally append
    // to leftover bytes from an earlier failed attempt.
    if part_path.exists() {
        let _ = tokio::fs::remove_file(&part_path).await;
    }

    log::debug!("{log_prefix} GET {url} -> {}", part_path.display());
    let client = reqwest::Client::builder()
        // 15s connect handshake; 30min overall request budget (covers 1.6 GB
        // GGML model on a 1 Mbps link). Per-chunk idle timeout is enforced
        // separately on each stream read below so half-open connections
        // fail fast instead of hanging the install task forever.
        .connect_timeout(Duration::from_secs(15))
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| format!("{log_prefix} build http client: {e}"))?;
    let started = Instant::now();
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("{log_prefix} request {url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "{log_prefix} non-2xx response from {url}: {}",
            resp.status()
        ));
    }
    let total = resp.content_length();
    log::debug!(
        "{log_prefix} response status={} content_length={:?}",
        resp.status(),
        total
    );

    let mut file = tokio::fs::File::create(&part_path)
        .await
        .map_err(|e| format!("{log_prefix} create {}: {e}", part_path.display()))?;
    let mut hasher = expected_sha256.is_some().then(Sha256::new);
    let mut downloaded: u64 = 0;
    let mut stream = resp.bytes_stream();
    loop {
        // Per-chunk idle timeout — if no bytes arrive within CHUNK_IDLE_TIMEOUT,
        // bail out so a stalled half-open TCP connection doesn't hold the install
        // task forever. Clean up the .part on the way out so a retry starts fresh.
        let next = tokio::time::timeout(CHUNK_IDLE_TIMEOUT, stream.next()).await;
        let chunk = match next {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(_) => {
                drop(file);
                let _ = tokio::fs::remove_file(&part_path).await;
                return Err(format!(
                    "{log_prefix} body stream idle for >{}s after {downloaded} bytes; aborting",
                    CHUNK_IDLE_TIMEOUT.as_secs()
                ));
            }
        };
        let bytes = match chunk {
            Ok(bytes) => bytes,
            Err(e) => {
                drop(file);
                let _ = tokio::fs::remove_file(&part_path).await;
                return Err(format!("{log_prefix} body stream: {e}"));
            }
        };
        if let Some(h) = hasher.as_mut() {
            h.update(&bytes);
        }
        if let Err(e) = file.write_all(&bytes).await {
            drop(file);
            let _ = tokio::fs::remove_file(&part_path).await;
            return Err(format!("{log_prefix} write {}: {e}", part_path.display()));
        }
        downloaded = downloaded.saturating_add(bytes.len() as u64);
        on_progress(downloaded, total);
    }
    file.flush()
        .await
        .map_err(|e| format!("{log_prefix} flush {}: {e}", part_path.display()))?;
    drop(file);

    if downloaded < min_bytes {
        let _ = tokio::fs::remove_file(&part_path).await;
        return Err(format!(
            "{log_prefix} downloaded payload too small: {downloaded} bytes < min {min_bytes}"
        ));
    }
    if let (Some(expected), Some(hasher)) = (expected_sha256, hasher) {
        let got = hex::encode(hasher.finalize());
        let expected_norm = expected.trim().to_ascii_lowercase();
        if got != expected_norm {
            // Never log the full file contents on mismatch — just the hashes.
            log::warn!(
                "{log_prefix} sha256 mismatch expected={} got={}",
                expected_norm,
                got
            );
            let _ = tokio::fs::remove_file(&part_path).await;
            return Err(format!(
                "{log_prefix} sha256 mismatch (expected {expected_norm}, got {got})"
            ));
        }
    }

    // Atomic rename — only after all checks pass. On Windows
    // `tokio::fs::rename` maps to `MoveFileExW` which fails if the dest
    // already exists, so remove it first.
    if dest.exists() {
        tokio::fs::remove_file(dest)
            .await
            .map_err(|e| format!("{log_prefix} remove existing {}: {e}", dest.display()))?;
    }
    tokio::fs::rename(&part_path, dest).await.map_err(|e| {
        format!(
            "{log_prefix} rename {} -> {}: {e}",
            part_path.display(),
            dest.display()
        )
    })?;
    log::debug!(
        "{log_prefix} downloaded {} bytes -> {} elapsed_ms={}",
        downloaded,
        dest.display(),
        started.elapsed().as_millis()
    );
    Ok(())
}

/// Produce the `.part` sibling of `dest`. Helper kept testable.
pub fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_os_string();
    s.push(".part");
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_path_appends_part_suffix() {
        let p = part_path(Path::new("/tmp/foo.bin"));
        assert_eq!(
            p.file_name().unwrap().to_string_lossy(),
            "foo.bin.part",
            "should append .part"
        );
    }

    #[test]
    fn part_path_handles_no_extension() {
        let p = part_path(Path::new("/tmp/binaryname"));
        assert_eq!(p.file_name().unwrap().to_string_lossy(), "binaryname.part");
    }

    #[test]
    fn voice_install_state_as_str_is_stable() {
        // The UI relies on the lowercase string form — guard against an
        // accidental rename breaking the wire contract.
        assert_eq!(VoiceInstallState::Missing.as_str(), "missing");
        assert_eq!(VoiceInstallState::Installing.as_str(), "installing");
        assert_eq!(VoiceInstallState::Installed.as_str(), "installed");
        assert_eq!(VoiceInstallState::Broken.as_str(), "broken");
        assert_eq!(VoiceInstallState::Error.as_str(), "error");
    }

    #[test]
    fn read_status_defaults_to_missing_for_unseen_engine() {
        let unique = format!("test-engine-{}", uuid::Uuid::new_v4());
        let snapshot = read_status(&unique);
        assert_eq!(snapshot.state, VoiceInstallState::Missing);
        assert_eq!(snapshot.engine, unique);
        assert!(snapshot.progress.is_none());
    }

    #[test]
    fn write_and_read_status_roundtrip() {
        let engine = format!("rt-{}", uuid::Uuid::new_v4());
        let status = VoiceInstallStatus {
            engine: engine.clone(),
            state: VoiceInstallState::Installing,
            progress: Some(42),
            downloaded_bytes: Some(1024),
            total_bytes: Some(2048),
            stage: Some("downloading model".to_string()),
            error_detail: None,
        };
        write_status(status);
        let got = read_status(&engine);
        assert_eq!(got.state, VoiceInstallState::Installing);
        assert_eq!(got.progress, Some(42));
        assert_eq!(got.stage.as_deref(), Some("downloading model"));
        // Clean up so the suite stays deterministic for parallel runs.
        reset_status(&engine);
    }

    #[test]
    fn reset_status_returns_engine_to_missing() {
        let engine = format!("rs-{}", uuid::Uuid::new_v4());
        write_status(VoiceInstallStatus {
            engine: engine.clone(),
            state: VoiceInstallState::Installed,
            progress: None,
            downloaded_bytes: None,
            total_bytes: None,
            stage: None,
            error_detail: None,
        });
        reset_status(&engine);
        assert_eq!(read_status(&engine).state, VoiceInstallState::Missing);
    }

    #[tokio::test]
    async fn download_to_file_rejects_oversize_min_bytes() {
        // 4xx-like guard: a non-existent host fails before we can write
        // anything. Use a localhost port that nothing is listening on so
        // the test is hermetic.
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("never.bin");
        let result = download_to_file(
            "http://127.0.0.1:1/never",
            &dest,
            None,
            10,
            "[voice-install:test]",
            |_, _| {},
        )
        .await;
        assert!(result.is_err(), "expected network error on unused port");
        // No `.part` should be left behind on a connection failure.
        let part = part_path(&dest);
        assert!(
            !part.exists(),
            "no part file should remain after pre-stream failure"
        );
    }

    #[tokio::test]
    async fn download_to_file_streams_and_renames_atomically() {
        // Spin up a one-shot in-process server with hyper via reqwest's
        // test infrastructure isn't available here, so we stand up a tiny
        // TCP listener that serves a fixed body. Keep the body small so
        // the test stays fast.
        use std::io::Write as _;
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let body = b"hello voice-install body";
        let server = tokio::task::spawn_blocking(move || {
            let (mut sock, _) = listener.accept().unwrap();
            // Drain request bytes — we only need headers.
            let mut buf = [0u8; 1024];
            use std::io::Read as _;
            let _ = sock.read(&mut buf);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                body.len()
            );
            sock.write_all(response.as_bytes()).unwrap();
            sock.write_all(body).unwrap();
            sock.flush().unwrap();
        });

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("hello.bin");
        let url = format!("http://{addr}/hello");
        let mut last_progress = (0u64, None);
        let result = download_to_file(
            &url,
            &dest,
            None,
            5,
            "[voice-install:test]",
            |downloaded, total| {
                last_progress = (downloaded, total);
            },
        )
        .await;
        server.await.unwrap();
        assert!(result.is_ok(), "download failed: {result:?}");
        let on_disk = tokio::fs::read(&dest).await.unwrap();
        assert_eq!(on_disk.as_slice(), body, "wrong bytes landed on disk");
        assert!(last_progress.0 > 0, "progress callback should fire");
        assert!(
            !part_path(&dest).exists(),
            "part file should be renamed away"
        );
    }
}
