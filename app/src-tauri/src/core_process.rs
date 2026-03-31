use std::path::PathBuf;
use std::sync::Arc;

use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoreRunMode {
    InProcess,
    ChildProcess,
}

#[derive(Clone)]
pub struct CoreProcessHandle {
    child: Arc<Mutex<Option<Child>>>,
    task: Arc<Mutex<Option<JoinHandle<()>>>>,
    port: u16,
    core_bin: Option<PathBuf>,
    run_mode: CoreRunMode,
}

impl CoreProcessHandle {
    pub fn new(port: u16, core_bin: Option<PathBuf>, run_mode: CoreRunMode) -> Self {
        Self {
            child: Arc::new(Mutex::new(None)),
            task: Arc::new(Mutex::new(None)),
            port,
            core_bin,
            run_mode,
        }
    }

    pub fn rpc_url(&self) -> String {
        format!("http://127.0.0.1:{}/rpc", self.port)
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
            return Ok(());
        }

        match self.run_mode {
            CoreRunMode::InProcess => {
                log::warn!(
                    "[core] in-process core mode is unavailable in host-only build; falling back to child process"
                );
                let mut guard = self.child.lock().await;
                if guard.is_none() {
                    let mut cmd = if let Some(core_bin) = &self.core_bin {
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
                    let child = cmd
                        .spawn()
                        .map_err(|e| format!("failed to spawn core process: {e}"))?;
                    *guard = Some(child);
                }
            }
            CoreRunMode::ChildProcess => {
                let mut guard = self.child.lock().await;
                if guard.is_none() {
                    let mut cmd = if let Some(core_bin) = &self.core_bin {
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

    /// Stop the core process this handle spawned (child or in-process task). Safe to call if
    /// nothing was spawned or core was already external.
    pub async fn shutdown(&self) {
        let mut child_guard = self.child.lock().await;
        if let Some(mut child) = child_guard.take() {
            log::info!("[core] terminating child core process on app shutdown");
            if let Err(e) = child.kill().await {
                log::warn!("[core] failed to kill child core process: {e}");
            }
        }
        let mut task_guard = self.task.lock().await;
        if let Some(task) = task_guard.take() {
            task.abort();
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

    // Dev ergonomics: allow an explicit staged sidecar from src-tauri/binaries in
    // debug builds before falling back to self-subcommand spawning.
    if cfg!(debug_assertions) {
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
                let matches = file_name.starts_with("openhuman-core-") && file_name.ends_with(".exe");
                #[cfg(not(windows))]
                let matches = file_name.starts_with("openhuman-core-");
                if matches {
                    return Some(path);
                }
            }
        }

        return None;
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
            let matches =
                file_name.starts_with("openhuman-core-") || file_name.starts_with("openhuman-core-");

            if matches && !same_executable_path(&path, &exe) {
                return Some(path);
            }
        }
    }

    None
}
