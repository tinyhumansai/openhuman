//! In-process core lifecycle.
//!
//! The core's HTTP/JSON-RPC server runs as a tokio task inside the Tauri host
//! so its lifetime is tied to the GUI process — there is no sidecar to leak
//! on Cmd+Q.
//!
//! Stale-listener policy (see issue #1130): if something is already listening
//! on the configured port when `ensure_running` runs, we probe `GET /` to see
//! whether it is an OpenHuman core. If it is, we treat it as a stale process
//! left behind by a previous build/dev session and proactively terminate it
//! (graceful signal, then a force-kill that *revalidates* the pid is still
//! the same listener — guards against PID reuse if the original exits inside
//! the grace window) before spawning a fresh embedded server — otherwise the
//! new UI would silently bind to an older RPC implementation. If the listener
//! is something else (or unreachable), we refuse to attach and surface the
//! conflict so it can be diagnosed instead of producing 401s and version
//! drift downstream.
//! Set `OPENHUMAN_CORE_REUSE_EXISTING=1` to opt back into the legacy
//! attach-to-whatever-is-listening behavior (e.g. a manual `openhuman-core
//! run` harness for debugging).

use std::sync::Arc;
use std::sync::LazyLock;

use parking_lot::RwLock;
use tokio::net::TcpStream;
use tokio::sync::oneshot::error::TryRecvError;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};
use tokio_util::sync::CancellationToken;

use crate::process_kill::{kill_pid_force, kill_pid_term};

const EMBEDDED_CORE_READY_WAIT_ATTEMPTS: u16 = 200;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortFallbackNotice {
    pub preferred_port: u16,
    pub chosen_port: u16,
}

#[derive(Clone)]
pub struct CoreProcessHandle {
    task: Arc<Mutex<Option<JoinHandle<anyhow::Result<()>>>>>,
    shutdown_token: Arc<Mutex<CancellationToken>>,
    restart_lock: Arc<Mutex<()>>,
    preferred_port: u16,
    active_port: Arc<RwLock<u16>>,
    last_port_fallback: Arc<RwLock<Option<PortFallbackNotice>>>,
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
            shutdown_token: Arc::new(Mutex::new(CancellationToken::new())),
            restart_lock: Arc::new(Mutex::new(())),
            preferred_port: port,
            active_port: Arc::new(RwLock::new(port)),
            last_port_fallback: Arc::new(RwLock::new(None)),
            rpc_token: Arc::new(rpc_token),
        }
    }

    /// The bearer token the embedded core validates on inbound RPC requests.
    pub fn rpc_token(&self) -> &str {
        &self.rpc_token
    }

    pub fn rpc_url(&self) -> String {
        format!("http://127.0.0.1:{}/rpc", self.port())
    }

    pub fn port(&self) -> u16 {
        *self.active_port.read()
    }

    pub fn take_last_port_fallback_notice(&self) -> Option<PortFallbackNotice> {
        self.last_port_fallback.write().take()
    }

    /// Acquire the restart lock to serialize overlapping restart requests.
    pub async fn restart_lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.restart_lock.lock().await
    }

    async fn is_rpc_port_open(&self) -> bool {
        is_port_open(self.port()).await
    }

    pub async fn ensure_running(&self) -> Result<(), String> {
        // Idempotent fast path: if we already spawned the embedded server in
        // *this* process and it's still alive on the port, the listener is
        // us — return Ok without identifying or taking over. Without this,
        // a second `start_core_process` call (e.g. HMR re-mounting the boot
        // gate) sees its own port as bound, classifies the listener as
        // "stale OpenHuman", and walks into the SIGTERM/SIGKILL takeover
        // path against itself. (#1130 takeover is meant to recover from
        // *external* leftover binaries, not our own in-process spawn.)
        {
            let guard = self.task.lock().await;
            if let Some(task) = guard.as_ref() {
                if !task.is_finished() && self.is_rpc_port_open().await {
                    log::debug!(
                        "[core] ensure_running: embedded task already running on port {} — no-op",
                        self.port()
                    );
                    return Ok(());
                }
            }
        }

        if is_port_open(self.preferred_port).await {
            // Idempotent fast-path: if we already own a running embedded
            // task, the listener on this port is us — not a stale external
            // process. Without this short-circuit, a second `ensure_running`
            // call (from BootCheckGate re-render, React StrictMode mount, or
            // any double-invoke of `start_core_process`) hits the
            // `identify_listener` path, identifies the listener as
            // OpenHuman, calls `takeover_stale_listener`, and aborts with
            // "stale-listener pid <self> matches the Tauri host pid;
            // refusing to self-terminate". (#1316 introduced the
            // frontend-driven `start_core_process` invoke without
            // hardening `ensure_running` against double-invoke.)
            {
                let guard = self.task.lock().await;
                if let Some(task) = guard.as_ref() {
                    if !task.is_finished() {
                        log::debug!(
                            "[core] ensure_running: embedded task already running on port {}, returning Ok (idempotent)",
                            self.port()
                        );
                        return Ok(());
                    }
                }
            }

            if reuse_existing_listener_enabled() {
                log::warn!(
                    "[core] OPENHUMAN_CORE_REUSE_EXISTING=1 — attaching to whatever is listening on port {} without identification (legacy behavior)",
                    self.preferred_port
                );
                return Ok(());
            }

            match identify_listener(self.preferred_port).await {
                ListenerKind::OpenHuman => {
                    log::warn!(
                        "[core] found stale OpenHuman listener on port {} — taking over (issue #1130)",
                        self.preferred_port
                    );
                    self.takeover_stale_listener().await?;
                    // Fall through to spawn-and-wait below.
                }
                ListenerKind::Unknown { reason } => {
                    if is_expected_port_clash(&reason) {
                        log::warn!(
                            "[core] preferred RPC port {} is occupied by non-OpenHuman listener ({reason}); attempting fallback bind range",
                            self.preferred_port
                        );
                    } else {
                        log::error!(
                            "[core] preferred RPC port {} occupied by unexpected listener ({reason}); attempting fallback bind range",
                            self.preferred_port
                        );
                    }
                }
            }
        }

        for startup_attempt in 0..=1u8 {
            let mut retry_after_takeover = false;
            let shutdown_token = self.fresh_shutdown_token().await;
            let (ready_tx, mut ready_rx) = tokio::sync::oneshot::channel::<
                openhuman_core::core::jsonrpc::EmbeddedReadySignal,
            >();
            let mut received_ready = false;

            {
                let mut guard = self.task.lock().await;
                if guard.is_none() {
                    let port = self.preferred_port;
                    // Set OPENHUMAN_CORE_TOKEN as a process-global env var before
                    // spawning the embedded server. Same-process tokio task reads
                    // the same env, matching what a child sidecar would have
                    // received via Command::env.
                    std::env::set_var("OPENHUMAN_CORE_TOKEN", self.rpc_token.as_str());
                    // Surface the Tauri shell version to the in-process core so
                    // backend-bound HTTP requests can attach `x-tauri-version`
                    // analytics headers alongside `x-core-version`.
                    std::env::set_var("OPENHUMAN_TAURI_VERSION", env!("CARGO_PKG_VERSION"));
                    *self.active_port.write() = port;
                    *self.last_port_fallback.write() = None;

                    // Debug-build only: surface the RPC bearer token at a known
                    // tmpdir path so the e2e test runner (a separate Node process)
                    // can authenticate against the in-process core. Release builds
                    // never write this file. The test harness reads it from
                    // ${tmpdir}/openhuman-e2e-rpc-token.
                    //
                    // Token file is owner-read-write only (mode 0600) on Unix so a
                    // shared dev box doesn't leak the bearer to other local users.
                    #[cfg(debug_assertions)]
                    {
                        use std::io::Write as _;
                        let token_path = std::env::temp_dir().join("openhuman-e2e-rpc-token");
                        let write_result = (|| -> std::io::Result<()> {
                            let mut options = std::fs::OpenOptions::new();
                            options.create(true).write(true).truncate(true);
                            #[cfg(unix)]
                            {
                                use std::os::unix::fs::OpenOptionsExt as _;
                                options.mode(0o600);
                            }
                            let mut file = options.open(&token_path)?;
                            file.write_all(self.rpc_token.as_bytes())?;
                            file.sync_all()?;
                            Ok(())
                        })();
                        if let Err(err) = write_result {
                            log::warn!(
                                "[core] failed to write e2e token file at {}: {err}",
                                token_path.display()
                            );
                        } else {
                            log::debug!(
                                "[core] wrote e2e token file at {} (debug build only)",
                                token_path.display()
                            );
                        }
                    }
                    log::info!(
                        "[core] spawning embedded in-process core server on preferred port {port}"
                    );
                    let task = tokio::spawn(async move {
                        openhuman_core::core::jsonrpc::run_server_embedded_with_ready(
                            None,
                            Some(port),
                            true,
                            shutdown_token,
                            ready_tx,
                        )
                        .await
                    });
                    *guard = Some(task);
                    // Publish only after the embedded server has been spawned
                    // with OPENHUMAN_CORE_TOKEN in scope.
                    *CURRENT_RPC_TOKEN.write() = Some(self.rpc_token.to_string());
                    log::debug!("[auth] CURRENT_RPC_TOKEN set after embedded spawn");
                }
            }

            // Readiness budget: 200 iterations x 100ms = 20s. The embedded
            // core's JSON-RPC controller registry has grown over time and
            // earlier 4s/10s budgets started flaking under CI worker load
            // (issue: core_process tests intermittently failing with
            // "core process did not become ready"), especially under
            // cargo-llvm-cov instrumentation where the binary runs ~2x
            // slower. Normal runs still exit the loop as soon as the ready
            // signal arrives and the listener is open.
            for _ in 0..EMBEDDED_CORE_READY_WAIT_ATTEMPTS {
                if !received_ready {
                    match ready_rx.try_recv() {
                        Ok(ready_signal) => {
                            self.apply_embedded_ready_signal(ready_signal);
                            received_ready = true;
                        }
                        Err(TryRecvError::Empty) => {}
                        Err(TryRecvError::Closed) => {}
                    }
                }

                if received_ready && self.is_rpc_port_open().await {
                    log::info!("[core] core rpc became ready at {}", self.rpc_url());
                    return Ok(());
                }

                let mut guard = self.task.lock().await;
                if let Some(task) = guard.as_ref() {
                    if task.is_finished() {
                        let task = guard.take().expect("checked is_some");
                        drop(guard);
                        return match task.await {
                            Ok(Ok(())) => {
                                Err("in-process core server exited before becoming ready"
                                    .to_string())
                            }
                            Ok(Err(err)) => {
                                if let Some(openhuman_core::openhuman::connectivity::rpc::PickListenPortError::WouldTakeOver { preferred, .. }) = err
                                    .downcast_ref::<openhuman_core::openhuman::connectivity::rpc::PickListenPortError>()
                                {
                                    if startup_attempt == 0 {
                                        log::warn!(
                                            "[core] preferred port {} requested stale-listener takeover from embedded bind path; retrying once",
                                            preferred
                                        );
                                        self.takeover_stale_listener().await?;
                                        retry_after_takeover = true;
                                        break;
                                    }
                                }
                                Err(format!(
                                    "in-process core server exited before becoming ready: {err}"
                                ))
                            }
                            Err(err) => Err(format!(
                                "in-process core server task failed before ready: {err}"
                            )),
                        };
                    }
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }

            if retry_after_takeover {
                continue;
            }
            return Err("core process did not become ready".to_string());
        }

        Err("core process did not become ready".to_string())
    }

    fn apply_embedded_ready_signal(
        &self,
        ready: openhuman_core::core::jsonrpc::EmbeddedReadySignal,
    ) {
        *self.active_port.write() = ready.port;
        std::env::set_var("OPENHUMAN_CORE_RPC_URL", self.rpc_url());
        if let Some(preferred) = ready.fallback_from {
            let message = format!("port_fallback_engaged: {preferred} -> {}", ready.port);
            log::warn!("[core] {message}");
            sentry::add_breadcrumb(sentry::Breadcrumb {
                category: Some("core.port".to_string()),
                level: sentry::Level::Warning,
                message: Some(message),
                ..Default::default()
            });
            *self.last_port_fallback.write() = Some(PortFallbackNotice {
                preferred_port: preferred,
                chosen_port: ready.port,
            });
        } else {
            *self.last_port_fallback.write() = None;
        }
    }

    /// Identify the OS pid currently bound to our port and terminate it,
    /// then wait for the port to free. Used when the listener has been
    /// fingerprinted as an OpenHuman core (via `GET /`) so killing it is safe.
    async fn takeover_stale_listener(&self) -> Result<(), String> {
        let port = self.preferred_port;
        let pid = match find_pid_on_port(port) {
            Some(pid) => pid,
            None => {
                return Err(format!(
                    "could not determine pid bound to port {} — refusing to take over",
                    port
                ));
            }
        };
        let self_pid = std::process::id();
        if pid == self_pid {
            // Defensive — `ensure_running` checks the port before spawning,
            // so this branch should be unreachable. If it ever hits, killing
            // ourselves would be catastrophic.
            return Err(format!(
                "stale-listener pid {pid} matches the Tauri host pid; refusing to self-terminate"
            ));
        }
        log::warn!(
            "[core] terminating stale OpenHuman process pid={pid} on port {} (issue #1130)",
            port
        );
        if let Err(e) = kill_pid_term(pid) {
            return Err(format!("failed to signal stale openhuman pid {pid}: {e}"));
        }

        // Wait for the graceful exit, then revalidate ownership before any
        // force-kill — between the SIGTERM and a delayed SIGKILL the original
        // pid could have exited and been reused by an unrelated process. If
        // the port is now bound to a different pid (or to nothing), we do
        // NOT escalate to a force-kill against the originally-resolved pid.
        // (CodeRabbit feedback on #1166.)
        const GRACE_MS: u64 = 750;
        tokio::time::sleep(Duration::from_millis(GRACE_MS)).await;

        if is_port_open(port).await {
            match find_pid_on_port(port) {
                Some(current) if current == pid => {
                    log::warn!(
                        "[core] pid {pid} still bound to port {} after SIGTERM — escalating to SIGKILL",
                        port
                    );
                    if let Err(e) = kill_pid_force(pid) {
                        return Err(format!(
                            "failed to force-kill stale openhuman pid {pid}: {e}"
                        ));
                    }
                }
                Some(current) => {
                    return Err(format!(
                        "port {} rebounded to pid {current} after terminating pid {pid}; refusing to kill a different process",
                        port
                    ));
                }
                None => {
                    // Port still showed open in `is_port_open` but pid lookup
                    // returned nothing — likely a transient race with the
                    // listener tearing down. Fall through to the poll loop.
                }
            }
        }

        const POLL_MS: u64 = 100;
        const MAX_WAIT_MS: u64 = 5_000;
        let mut waited_ms: u64 = GRACE_MS;
        while is_port_open(port).await {
            if waited_ms >= MAX_WAIT_MS {
                return Err(format!(
                    "signaled pid {pid} but port {} remained bound after {MAX_WAIT_MS}ms",
                    port
                ));
            }
            tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
            waited_ms += POLL_MS;
        }
        log::info!(
            "[core] stale listener cleared (pid={pid}, port={}) after {waited_ms}ms",
            port
        );
        Ok(())
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
        if !had_managed_task {
            log::debug!(
                "[core] restart: no managed embedded task was running; ensure_running will resolve ownership/fallback"
            );
        }

        const POLL_MS: u64 = 50;
        const MAX_WAIT_MS: u64 = 10_000;
        let mut waited_ms: u64 = 0;
        while self.is_rpc_port_open().await {
            if waited_ms >= MAX_WAIT_MS {
                return Err(format!(
                    "Core RPC port {} did not become free after stopping the embedded server.",
                    self.port()
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

    async fn fresh_shutdown_token(&self) -> CancellationToken {
        let mut guard = self.shutdown_token.lock().await;
        if guard.is_cancelled() {
            log::debug!("[core] resetting embedded core shutdown token for new spawn");
            *guard = CancellationToken::new();
        }
        guard.clone()
    }

    async fn cancel_shutdown_token(&self, log_context: &str) {
        let token = self.shutdown_token.lock().await.clone();
        if token.is_cancelled() {
            log::debug!("[core] embedded core shutdown token already cancelled{log_context}");
        } else {
            log::info!("[core] cancelling embedded core shutdown token{log_context}");
            token.cancel();
        }
    }

    #[cfg(test)]
    async fn shutdown_token_is_cancelled(&self) -> bool {
        self.shutdown_token.lock().await.is_cancelled()
    }

    /// Stop the embedded server task. Safe to call when nothing is running.
    pub async fn shutdown(&self) {
        self.cancel_shutdown_token("").await;
        let task = {
            let mut task_guard = self.task.lock().await;
            task_guard.take()
        };
        let Some(mut task) = task else {
            return;
        };

        match timeout(Duration::from_secs(5), &mut task).await {
            Ok(Ok(Ok(()))) => {
                log::info!("[core] embedded core server task stopped gracefully");
            }
            Ok(Ok(Err(err))) => {
                log::warn!("[core] embedded core server task ended during shutdown: {err}");
            }
            Ok(Err(err)) => {
                log::warn!("[core] embedded core server task join failed during shutdown: {err}");
            }
            Err(_) => {
                log::warn!(
                    "[core] graceful embedded core shutdown timed out; aborting server task"
                );
                task.abort();
                let _ = task.await;
            }
        }
    }

    /// Synchronous-friendly shutdown for `RunEvent::ExitRequested`.
    ///
    /// Aborts the embedded server task so any background tokio tasks the
    /// server spawned stop driving I/O before CEF's teardown runs. Cheap
    /// and non-blocking on the UI thread — `JoinHandle::abort` returns
    /// immediately.
    pub async fn send_terminate_signal(&self) {
        self.cancel_shutdown_token(" on app shutdown").await;
        self.abort_task(" on app shutdown").await;
    }
}

pub fn default_core_port() -> u16 {
    std::env::var("OPENHUMAN_CORE_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(7788)
}

/// Whether `OPENHUMAN_CORE_REUSE_EXISTING` is set to a truthy value. Opts
/// back into the pre-#1130 behavior of attaching to whatever is listening
/// on the port without identification — useful for manual harnesses.
pub(crate) fn reuse_existing_listener_enabled() -> bool {
    std::env::var("OPENHUMAN_CORE_REUSE_EXISTING")
        .map(|v| matches!(v.trim(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

async fn is_port_open(port: u16) -> bool {
    matches!(
        timeout(
            Duration::from_millis(150),
            TcpStream::connect(("127.0.0.1", port)),
        )
        .await,
        Ok(Ok(_))
    )
}

/// What is currently listening on the core RPC port.
#[derive(Debug)]
enum ListenerKind {
    /// `GET /` returned a JSON body with `"name": "openhuman"` — i.e. a
    /// stale OpenHuman core process from a previous build/session.
    OpenHuman,
    /// Either the listener didn't speak HTTP, didn't respond, or returned
    /// a body that doesn't identify as openhuman.
    Unknown { reason: String },
}

/// Probe `GET http://127.0.0.1:<port>/` to fingerprint the listener.
/// Unauthenticated — the core's root handler does not require a token.
async fn identify_listener(port: u16) -> ListenerKind {
    let url = format!("http://127.0.0.1:{port}/");
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_millis(750))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return ListenerKind::Unknown {
                reason: format!("reqwest client build failed: {e}"),
            };
        }
    };
    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            return ListenerKind::Unknown {
                reason: format!("probe GET / failed: {e}"),
            };
        }
    };
    if !resp.status().is_success() {
        return ListenerKind::Unknown {
            reason: format!("probe GET / returned status {}", resp.status()),
        };
    }
    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => {
            return ListenerKind::Unknown {
                reason: format!("probe GET / body read failed: {e}"),
            };
        }
    };
    if is_openhuman_root_body(&body) {
        log::info!("[core] listener on port {port} identified as openhuman core");
        ListenerKind::OpenHuman
    } else {
        let preview: String = body.chars().take(80).collect();
        ListenerKind::Unknown {
            reason: format!("probe GET / body did not identify as openhuman ({preview:?})"),
        }
    }
}

/// Pure parse of the root-handler JSON. Public-by-test so the fingerprinting
/// logic stays unit-testable without standing up an HTTP server.
fn is_openhuman_root_body(body: &str) -> bool {
    let value: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return false,
    };
    value
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s == "openhuman")
        .unwrap_or(false)
}

/// Returns true when a port conflict is deterministic environment state, not
/// a high-signal unknown squatter worth sending to Sentry at error level.
fn is_expected_port_clash(reason: &str) -> bool {
    let reason = reason.to_ascii_lowercase();
    reason.contains("error sending request for url")
        || reason.contains("connection refused")
        || reason.contains("returned status 404")
        || reason.contains("returned status 200")
        || reason.contains("body did not identify as openhuman")
        || reason.contains("already in use by another process")
        || reason.contains("os error 10013")
        || reason.contains("wsaeacces")
}

#[cfg(unix)]
fn find_pid_on_port(port: u16) -> Option<u32> {
    let output = std::process::Command::new("lsof")
        .args(["-nP", "-iTCP", &format!("-i:{port}"), "-sTCP:LISTEN", "-t"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_lsof_pid(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(windows)]
fn find_pid_on_port(port: u16) -> Option<u32> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let output = std::process::Command::new("netstat")
        .args(["-ano", "-p", "TCP"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_netstat_pid(&String::from_utf8_lossy(&output.stdout), port)
}

/// Pure parse of `lsof -t` output (one pid per line; first wins).
fn parse_lsof_pid(stdout: &str) -> Option<u32> {
    stdout
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .and_then(|l| l.parse::<u32>().ok())
}

/// Pure parse of `netstat -ano` output for a LISTENING entry on `port`.
#[allow(dead_code)] // exercised only on windows builds
fn parse_netstat_pid(stdout: &str, port: u16) -> Option<u32> {
    let needle = format!(":{port}");
    for line in stdout.lines() {
        let trimmed = line.trim();
        if !trimmed.contains("LISTENING") {
            continue;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        // Expected: ["TCP", "127.0.0.1:7788", "0.0.0.0:0", "LISTENING", "1234"]
        if parts.len() >= 5 && parts[1].ends_with(&needle) {
            if let Ok(pid) = parts[parts.len() - 1].parse::<u32>() {
                return Some(pid);
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "core_process_tests.rs"]
mod tests;
