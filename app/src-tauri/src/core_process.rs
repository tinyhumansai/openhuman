use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};

/// Propagate ANSI color hints to the spawned core child.
///
/// Core's tracing formatter auto-detects color via `stderr.is_terminal()`,
/// but when core runs as a grandchild under `yarn tauri dev` the inherited
/// stderr may not register as a TTY even though the ultimate terminal
/// supports ANSI. If the Tauri process itself is attached to a TTY we
/// forward `FORCE_COLOR=1` so core emits colored log lines; `NO_COLOR`
/// (user opt-out) always wins and short-circuits the propagation.
fn apply_core_color_env(cmd: &mut Command) {
    if std::env::var_os("NO_COLOR").is_some() {
        return;
    }
    if std::io::stderr().is_terminal() {
        cmd.env("FORCE_COLOR", "1");
    }
}

/// Hide the console window that Windows would otherwise allocate for the
/// core sidecar. The core binary is a console-subsystem executable so that
/// `openhuman core run` in a terminal behaves normally, but when the GUI
/// shell spawns it as a child a stray conhost window pops up on top of the
/// app. `CREATE_NO_WINDOW` suppresses that while leaving stdout/stderr
/// piping intact for our log forwarding.
#[cfg(windows)]
fn apply_core_no_window(cmd: &mut Command) {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn apply_core_no_window(_cmd: &mut Command) {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoreRunMode {
    InProcess,
    ChildProcess,
}

#[derive(Clone)]
pub struct CoreProcessHandle {
    child: Arc<Mutex<Option<Child>>>,
    task: Arc<Mutex<Option<JoinHandle<()>>>>,
    restart_lock: Arc<Mutex<()>>,
    port: u16,
    core_bin: Option<PathBuf>,
    /// Override path set by the auto-updater after staging a new binary.
    core_bin_override: Arc<Mutex<Option<PathBuf>>>,
    run_mode: CoreRunMode,
}

impl CoreProcessHandle {
    pub fn new(port: u16, core_bin: Option<PathBuf>, run_mode: CoreRunMode) -> Self {
        Self {
            child: Arc::new(Mutex::new(None)),
            task: Arc::new(Mutex::new(None)),
            restart_lock: Arc::new(Mutex::new(())),
            port,
            core_bin,
            core_bin_override: Arc::new(Mutex::new(None)),
            run_mode,
        }
    }

    pub fn rpc_url(&self) -> String {
        format!("http://127.0.0.1:{}/rpc", self.port)
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Replace the core binary path so that the next `ensure_running()` launches
    /// the new binary instead of the original one captured at construction time.
    pub async fn set_core_bin(&self, new_bin: PathBuf) {
        // We store it via a second field; but since core_bin is not behind a lock,
        // we work around this by swapping the entire handle's notion of what to launch.
        // For now, mutate through an interior-mutable wrapper.
        log::info!(
            "[core] set_core_bin: updating core binary path to {}",
            new_bin.display()
        );
        *self.core_bin_override.lock().await = Some(new_bin);
    }

    /// Resolve which binary to launch: override (set by `set_core_bin`) > original.
    async fn effective_core_bin(&self) -> Option<PathBuf> {
        let override_guard = self.core_bin_override.lock().await;
        if let Some(ref path) = *override_guard {
            return Some(path.clone());
        }
        self.core_bin.clone()
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
                "[core] reusing port {} — if channel/Telegram behavior mismatches the app, another stale `openhuman` core may be attached; check [core-update] logs for version skew.",
                self.port
            );
            return Ok(());
        }

        let effective_bin = self.effective_core_bin().await;

        match self.run_mode {
            CoreRunMode::InProcess => {
                log::warn!(
                    "[core] in-process core mode is unavailable in host-only build; falling back to child process"
                );
                let mut guard = self.child.lock().await;
                if guard.is_none() {
                    let mut cmd = if let Some(core_bin) = &effective_bin {
                        let mut cmd = Command::new(core_bin);
                        if is_current_exe_path(core_bin) {
                            cmd.arg("core");
                        }
                        cmd.arg("run").arg("--port").arg(self.port.to_string());
                        cmd
                    } else {
                        let exe = std::env::current_exe()
                            .map_err(|e| format!("failed to resolve current executable: {e}"))?;
                        let mut cmd = Command::new(exe);
                        cmd.arg("core")
                            .arg("run")
                            .arg("--port")
                            .arg(self.port.to_string());
                        cmd
                    };
                    apply_core_color_env(&mut cmd);
                    apply_core_no_window(&mut cmd);
                    let child = cmd
                        .spawn()
                        .map_err(|e| format!("failed to spawn core process: {e}"))?;
                    *guard = Some(child);
                }
            }
            CoreRunMode::ChildProcess => {
                let mut guard = self.child.lock().await;
                if guard.is_none() {
                    let mut cmd = if let Some(core_bin) = &effective_bin {
                        let mut cmd = Command::new(core_bin);
                        if is_current_exe_path(core_bin) {
                            // Safety: if core_bin resolves to this GUI executable, force the
                            // explicit subcommand path so we don't accidentally relaunch clients.
                            cmd.arg("core");
                        }
                        cmd.arg("run").arg("--port").arg(self.port.to_string());
                        log::info!(
                            "[core] spawning dedicated core binary: {:?} run --port {}",
                            cmd.as_std().get_program(),
                            self.port
                        );
                        cmd
                    } else {
                        let exe = std::env::current_exe()
                            .map_err(|e| format!("failed to resolve current executable: {e}"))?;
                        let mut cmd = Command::new(exe);
                        cmd.arg("core")
                            .arg("run")
                            .arg("--port")
                            .arg(self.port.to_string());
                        log::warn!(
                            "[core] dedicated core binary not found; falling back to self subcommand"
                        );
                        cmd
                    };

                    apply_core_color_env(&mut cmd);
                    apply_core_no_window(&mut cmd);
                    let child = cmd
                        .spawn()
                        .map_err(|e| format!("failed to spawn core process: {e}"))?;

                    *guard = Some(child);
                }
            }
        }

        for _ in 0..40 {
            if self.is_rpc_port_open().await {
                log::info!("[core] core rpc became ready at {}", self.rpc_url());
                return Ok(());
            }

            match self.run_mode {
                CoreRunMode::InProcess => {
                    let mut guard = self.task.lock().await;
                    if let Some(task) = guard.as_ref() {
                        if task.is_finished() {
                            let task = guard.take().expect("checked is_some");
                            drop(guard);
                            match task.await {
                                Ok(_) => {
                                    return Err(
                                        "in-process core server exited before becoming ready"
                                            .to_string(),
                                    )
                                }
                                Err(err) => {
                                    return Err(format!(
                                        "in-process core server task failed before ready: {err}"
                                    ))
                                }
                            }
                        }
                    }
                }
                CoreRunMode::ChildProcess => {
                    let mut guard = self.child.lock().await;
                    if let Some(child) = guard.as_mut() {
                        match child.try_wait() {
                            Ok(Some(status)) => {
                                return Err(format!("core process exited before ready: {status}"));
                            }
                            Ok(None) => {}
                            Err(e) => {
                                return Err(format!("failed checking core process status: {e}"));
                            }
                        }
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        Err("core process did not become ready".to_string())
    }

    /// Restart the core process to pick up updated macOS permission grants.
    ///
    /// macOS caches permission state per-process; the running sidecar never sees
    /// a newly granted permission until it restarts. This method shuts down the
    /// current child, waits until the RPC port is free (so `ensure_running` does not
    /// fast-return while the old listener is still bound), then spawns a fresh instance.
    ///
    /// If another process is listening on the core port (e.g. manual `openhuman core run`),
    /// shutdown does not stop it — we time out and return an error instead of a false success.
    ///
    /// Issue: <https://github.com/tinyhumansai/openhuman/issues/133>
    pub async fn restart(&self) -> Result<(), String> {
        log::info!("[core] restarting core process for permission refresh");

        let had_managed_child = {
            let guard = self.child.lock().await;
            guard.is_some()
        };
        log::debug!(
            "[core] restart: had_managed_child={} before shutdown",
            had_managed_child
        );

        self.shutdown().await;
        log::debug!(
            "[core] restart: shutdown complete, checking port {}",
            self.port
        );

        // If we never spawned the sidecar (something else was already listening), we cannot free
        // the port — fail fast with a clear message instead of polling for 8s.
        if !had_managed_child && self.is_rpc_port_open().await {
            log::error!(
                "[core] restart: no child to stop but port {} is open — another process owns it",
                self.port
            );
            return Err(format!(
                "Core RPC port {} is already in use by another process (OpenHuman did not start it). Quit any `openhuman core run` in a terminal or free the port, then relaunch the app. You can also set OPENHUMAN_CORE_PORT to a different port.",
                self.port
            ));
        }

        // After kill+wait on our child, the port should close; poll briefly in case the OS is slow
        // to release the socket.
        const POLL_MS: u64 = 50;
        const MAX_WAIT_MS: u64 = 10_000;
        let mut waited_ms: u64 = 0;
        while self.is_rpc_port_open().await {
            if waited_ms >= MAX_WAIT_MS {
                log::error!(
                    "[core] restart: port {} still in use after {}ms (had_managed_child={})",
                    self.port,
                    MAX_WAIT_MS,
                    had_managed_child
                );
                return Err(format!(
                    "Core RPC port {} did not become free after stopping the sidecar. Quit any other process using this port (e.g. `openhuman core run`) or change OPENHUMAN_CORE_PORT.",
                    self.port
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(POLL_MS)).await;
            waited_ms += POLL_MS;
        }

        log::debug!("[core] restart: port free, calling ensure_running");
        let result = self.ensure_running().await;
        match &result {
            Ok(()) => log::info!("[core] restart: core process ready after restart"),
            Err(e) => log::error!("[core] restart: failed to restart core process: {e}"),
        }
        result
    }

    /// Stop the core process this handle spawned (child or in-process task). Safe to call if
    /// nothing was spawned or core was already external.
    ///
    /// On Unix, sends SIGTERM first so the core process can run its graceful
    /// shutdown hooks (e.g. stopping the autocomplete engine and its Swift
    /// overlay helper). Falls back to SIGKILL after a timeout.
    pub async fn shutdown(&self) {
        let mut child_guard = self.child.lock().await;
        if let Some(mut child) = child_guard.take() {
            log::info!("[core] terminating child core process on app shutdown");

            let exited = self.try_graceful_terminate(&child).await;

            if !exited {
                log::info!("[core] graceful shutdown timed out, sending SIGKILL");
                if let Err(e) = child.kill().await {
                    log::warn!("[core] failed to kill child core process: {e}");
                }
            }

            // Wait for the process to exit so the RPC listen socket is released before restart
            // checks the port (otherwise we can spuriously hit "port still in use").
            match timeout(Duration::from_secs(12), child.wait()).await {
                Ok(Ok(status)) => {
                    log::debug!("[core] child core process reaped after shutdown: {status}");
                }
                Ok(Err(e)) => {
                    log::warn!("[core] wait on child core process after shutdown: {e}");
                }
                Err(_) => {
                    log::warn!("[core] timed out waiting for child core process to exit (12s)");
                }
            }
        }
        let mut task_guard = self.task.lock().await;
        if let Some(task) = task_guard.take() {
            task.abort();
        }
    }

    /// Send SIGTERM to the child and wait up to 5 seconds for it to exit.
    /// Returns `true` if the process exited gracefully, `false` if it's still
    /// alive (caller should escalate to SIGKILL).
    async fn try_graceful_terminate(&self, child: &Child) -> bool {
        #[cfg(unix)]
        {
            use nix::sys::signal::{self, Signal};
            use nix::unistd::Pid;

            let Some(pid) = child.id() else {
                log::debug!("[core] child has no PID (already exited?)");
                return true;
            };

            log::info!("[core] sending SIGTERM to core process (pid={pid})");
            if let Err(e) = signal::kill(Pid::from_raw(pid as i32), Signal::SIGTERM) {
                log::warn!("[core] failed to send SIGTERM: {e}");
                return false;
            }

            // Poll for exit for up to 5 seconds.
            const GRACE_PERIOD: Duration = Duration::from_secs(5);
            const POLL_INTERVAL: Duration = Duration::from_millis(100);
            let start = tokio::time::Instant::now();

            while start.elapsed() < GRACE_PERIOD {
                // Check if process is still alive (signal 0 = existence check).
                match signal::kill(Pid::from_raw(pid as i32), None) {
                    Err(nix::errno::Errno::ESRCH) => {
                        log::info!(
                            "[core] core process exited gracefully after SIGTERM ({}ms)",
                            start.elapsed().as_millis()
                        );
                        return true;
                    }
                    _ => {}
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }

            log::warn!(
                "[core] core process still alive after {}s grace period",
                GRACE_PERIOD.as_secs()
            );
            false
        }

        #[cfg(not(unix))]
        {
            // On non-Unix platforms, there is no SIGTERM equivalent; the caller
            // will use `child.kill()` directly.
            let _ = child;
            false
        }
    }
}

fn is_current_exe_path(candidate: &std::path::Path) -> bool {
    let Ok(current) = std::env::current_exe() else {
        return false;
    };
    same_executable_path(candidate, &current)
}

fn same_executable_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a_real), Ok(b_real)) => a_real == b_real,
        _ => false,
    }
}

pub fn default_core_port() -> u16 {
    std::env::var("OPENHUMAN_CORE_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(7788)
}

pub fn default_core_run_mode(_daemon_mode: bool) -> CoreRunMode {
    if let Ok(value) = std::env::var("OPENHUMAN_CORE_RUN_MODE") {
        let normalized = value.trim().to_ascii_lowercase();
        if matches!(normalized.as_str(), "inprocess" | "in-process" | "internal") {
            return CoreRunMode::InProcess;
        }
        if matches!(
            normalized.as_str(),
            "child" | "process" | "external" | "sidecar"
        ) {
            return CoreRunMode::ChildProcess;
        }
    }

    // Default to a dedicated core process so app and core lifecycles are separated.
    CoreRunMode::ChildProcess
}

pub fn default_core_bin() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("OPENHUMAN_CORE_BIN") {
        let candidate = PathBuf::from(path);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    // Dev: prefer a staged sidecar under src-tauri/binaries, then use the same search as
    // release (next to the .app, Resources/, etc.). Previously we returned None here when the
    // folder was empty, which forced `core run` on the GUI binary — a different TCC identity than
    // `openhuman-core-*` and misleading "still denied" after granting the sidecar name.
    #[cfg(debug_assertions)]
    {
        let binaries_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries");
        if let Ok(entries) = std::fs::read_dir(&binaries_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                #[cfg(windows)]
                let matches =
                    file_name.starts_with("openhuman-core-") && file_name.ends_with(".exe");
                #[cfg(not(windows))]
                let matches = file_name.starts_with("openhuman-core-");
                if matches {
                    return Some(path);
                }
            }
        }
    }

    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;

    #[cfg(windows)]
    let standalone = exe_dir.join("openhuman-core.exe");
    #[cfg(not(windows))]
    let standalone = exe_dir.join("openhuman-core");

    if standalone.exists() && !same_executable_path(&standalone, &exe) {
        return Some(standalone);
    }

    #[cfg(windows)]
    let legacy_standalone = exe_dir.join("openhuman-core.exe");
    #[cfg(not(windows))]
    let legacy_standalone = exe_dir.join("openhuman-core");

    if legacy_standalone.exists() && !same_executable_path(&legacy_standalone, &exe) {
        return Some(legacy_standalone);
    }

    // Sidecar layout: bundle.externalBin("binaries/openhuman-core") is emitted as
    // openhuman-core-<target-triple>(.exe) under app resources.
    let search_dirs = {
        let mut dirs = vec![exe_dir.to_path_buf()];
        #[cfg(target_os = "macos")]
        {
            if let Some(resources_dir) = exe_dir.parent().map(|p| p.join("Resources")) {
                dirs.push(resources_dir);
            }
        }
        dirs
    };

    for dir in search_dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };

            #[cfg(windows)]
            let matches = (file_name.starts_with("openhuman-core-") && file_name.ends_with(".exe"))
                || (file_name.starts_with("openhuman-core-") && file_name.ends_with(".exe"));
            #[cfg(not(windows))]
            let matches = file_name.starts_with("openhuman-core-")
                || file_name.starts_with("openhuman-core-");

            if matches && !same_executable_path(&path, &exe) {
                return Some(path);
            }
        }
    }

    None
}

#[cfg(test)]
#[path = "core_process_tests.rs"]
mod tests;
