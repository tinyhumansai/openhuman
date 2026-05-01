//! In-process core lifecycle.
//!
//! The core's HTTP/JSON-RPC server runs as a tokio task inside the Tauri host
//! so its lifetime is tied to the GUI process — there is no sidecar to leak
//! on Cmd+Q. If something is already listening on the configured port (e.g.
//! a manual `openhuman-core run` harness for debugging), `ensure_running`
//! attaches to it instead of spawning a duplicate listener.

use std::sync::Arc;
use std::sync::LazyLock;

use parking_lot::RwLock;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};

/// Generate a 256-bit cryptographically-random bearer token as a hex string.
///
/// Uses the same encoding as `openhuman_core::core::auth::generate_token`
/// (`hex::encode`) so the token format never silently diverges between the
/// Tauri-side generator and the core-side validator.
pub fn generate_rpc_token() -> String {
    use rand::RngCore as _;
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

static CURRENT_RPC_TOKEN: LazyLock<RwLock<Option<String>>> = LazyLock::new(|| RwLock::new(None));

pub fn current_rpc_token() -> Option<String> {
    CURRENT_RPC_TOKEN.read().clone()
}

#[derive(Clone)]
pub struct CoreProcessHandle {
    task: Arc<Mutex<Option<JoinHandle<()>>>>,
    restart_lock: Arc<Mutex<()>>,
    port: u16,
    /// Bearer token the embedded server validates on every inbound request.
    /// Passed to the embedded server through the `OPENHUMAN_CORE_TOKEN`
    /// process env var (set in `ensure_running` before spawn) and exposed to
    /// the frontend via the `core_rpc_token` Tauri command so every RPC call
    /// can include `Authorization: Bearer`.
    rpc_token: Arc<String>,
}

impl CoreProcessHandle {
    pub fn new(port: u16) -> Self {
        // CURRENT_RPC_TOKEN is intentionally NOT set here. It is published by
        // ensure_running() only after the embedded server has been spawned
        // with OPENHUMAN_CORE_TOKEN in scope. Setting it here would advertise
        // a token that an existing process listening on the port (the
        // harness-attach fast-path) has never seen, causing 401s on every
        // authenticated call.
        let rpc_token = generate_rpc_token();
        Self {
            task: Arc::new(Mutex::new(None)),
            restart_lock: Arc::new(Mutex::new(())),
            port,
            rpc_token: Arc::new(rpc_token),
        }
    }

    /// The bearer token the embedded core validates on inbound RPC requests.
    pub fn rpc_token(&self) -> &str {
        &self.rpc_token
    }

    pub fn rpc_url(&self) -> String {
        format!("http://127.0.0.1:{}/rpc", self.port)
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Acquire the restart lock to serialize overlapping restart requests.
    pub async fn restart_lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.restart_lock.lock().await
    }

    async fn is_rpc_port_open(&self) -> bool {
        matches!(
            timeout(
                Duration::from_millis(150),
                TcpStream::connect(("127.0.0.1", self.port)),
            )
            .await,
            Ok(Ok(_))
        )
    }

    pub async fn ensure_running(&self) -> Result<(), String> {
        if self.is_rpc_port_open().await {
            log::info!(
                "[core] found existing core rpc endpoint at {}",
                self.rpc_url()
            );
            log::warn!(
                "[core] reusing port {} — another `openhuman-core` instance is already listening; this Tauri host will not spawn an embedded server. Authenticated Tauri-side calls will 401 unless the listener was started with this process's OPENHUMAN_CORE_TOKEN.",
                self.port
            );
            return Ok(());
        }

        {
            let mut guard = self.task.lock().await;
            if guard.is_none() {
                let port = self.port;
                // Set OPENHUMAN_CORE_TOKEN as a process-global env var before
                // spawning the embedded server. Same-process tokio task reads
                // the same env, matching what a child sidecar would have
                // received via Command::env.
                std::env::set_var("OPENHUMAN_CORE_TOKEN", self.rpc_token.as_str());
                log::info!("[core] spawning embedded in-process core server on port {port}");
                let task = tokio::spawn(async move {
                    if let Err(e) =
                        openhuman_core::core::jsonrpc::run_server_embedded(None, Some(port), true)
                            .await
                    {
                        log::error!("[core] embedded core server exited with error: {e}");
                    } else {
                        log::info!("[core] embedded core server exited cleanly");
                    }
                });
                *guard = Some(task);
                // Publish only after the embedded server has been spawned
                // with OPENHUMAN_CORE_TOKEN in scope.
                *CURRENT_RPC_TOKEN.write() = Some(self.rpc_token.to_string());
                log::debug!("[auth] CURRENT_RPC_TOKEN set after embedded spawn");
            }
        }

        for _ in 0..40 {
            if self.is_rpc_port_open().await {
                log::info!("[core] core rpc became ready at {}", self.rpc_url());
                return Ok(());
            }

            let mut guard = self.task.lock().await;
            if let Some(task) = guard.as_ref() {
                if task.is_finished() {
                    let task = guard.take().expect("checked is_some");
                    drop(guard);
                    return match task.await {
                        Ok(_) => {
                            Err("in-process core server exited before becoming ready".to_string())
                        }
                        Err(err) => Err(format!(
                            "in-process core server task failed before ready: {err}"
                        )),
                    };
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        Err("core process did not become ready".to_string())
    }

    /// Restart the embedded core to pick up updated macOS permission grants.
    ///
    /// macOS caches permission state per-process; restarting forces a fresh
    /// read. If something else is bound to the port (e.g. a manual
    /// `openhuman-core run` harness) we surface that instead of looping.
    ///
    /// Issue: <https://github.com/tinyhumansai/openhuman/issues/133>
    pub async fn restart(&self) -> Result<(), String> {
        log::info!("[core] restarting embedded core server for permission refresh");

        let had_managed_task = {
            let guard = self.task.lock().await;
            guard.is_some()
        };

        self.shutdown().await;

        if !had_managed_task && self.is_rpc_port_open().await {
            log::error!(
                "[core] restart: nothing to stop but port {} is in use — another process owns it",
                self.port
            );
            return Err(format!(
                "Core RPC port {} is already in use by another process (OpenHuman did not start it). Quit any `openhuman-core run` in a terminal or set OPENHUMAN_CORE_PORT to a different port, then relaunch the app.",
                self.port
            ));
        }

        const POLL_MS: u64 = 50;
        const MAX_WAIT_MS: u64 = 10_000;
        let mut waited_ms: u64 = 0;
        while self.is_rpc_port_open().await {
            if waited_ms >= MAX_WAIT_MS {
                return Err(format!(
                    "Core RPC port {} did not become free after stopping the embedded server.",
                    self.port
                ));
            }
            tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
            waited_ms += POLL_MS;
        }

        let result = self.ensure_running().await;
        match &result {
            Ok(()) => log::info!("[core] restart: embedded core ready after restart"),
            Err(e) => log::error!("[core] restart: failed to restart embedded core: {e}"),
        }
        result
    }

    /// Lock the task slot, take its handle if any, and abort it. Shared by
    /// `shutdown` (cleanup-on-drop semantics) and `send_terminate_signal`
    /// (cooperative early teardown from `RunEvent::ExitRequested`).
    async fn abort_task(&self, log_context: &str) {
        let mut task_guard = self.task.lock().await;
        if let Some(task) = task_guard.take() {
            log::info!("[core] aborting embedded core server task{log_context}");
            task.abort();
        }
    }

    /// Stop the embedded server task. Safe to call when nothing is running.
    pub async fn shutdown(&self) {
        self.abort_task("").await;
    }

    /// Synchronous-friendly shutdown for `RunEvent::ExitRequested`.
    ///
    /// Aborts the embedded server task so any background tokio tasks the
    /// server spawned stop driving I/O before CEF's teardown runs. Cheap
    /// and non-blocking on the UI thread — `JoinHandle::abort` returns
    /// immediately.
    pub async fn send_terminate_signal(&self) {
        self.abort_task(" on app shutdown").await;
    }
}

pub fn default_core_port() -> u16 {
    std::env::var("OPENHUMAN_CORE_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(7788)
}

#[cfg(test)]
#[path = "core_process_tests.rs"]
mod tests;
