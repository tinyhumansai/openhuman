use std::path::PathBuf;
use std::sync::Arc;

use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

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

    pub async fn ensure_running(&self) -> Result<(), String> {
        if crate::core_rpc::ping().await {
            log::info!(
                "[core] found existing core rpc endpoint at {}",
                self.rpc_url()
            );
            return Ok(());
        }

        match self.run_mode {
            CoreRunMode::InProcess => {
                let mut guard = self.task.lock().await;
                if guard.is_none() {
                    let port = self.port;
                    log::info!("[core] launching in-process core server on port {}", port);
                    let task = tokio::spawn(async move {
                        if let Err(err) = openhuman_core::core_server::run_server(Some(port)).await
                        {
                            log::error!("[core] in-process core server exited with error: {err}");
                        } else {
                            log::warn!("[core] in-process core server exited");
                        }
                    });
                    *guard = Some(task);
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
                        cmd.arg("serve").arg("--port").arg(self.port.to_string());
                        log::info!(
                            "[core] spawning dedicated core binary: {:?} serve --port {}",
                            cmd.as_std().get_program(),
                            self.port
                        );
                        cmd
                    } else {
                        let exe = std::env::current_exe()
                            .map_err(|e| format!("failed to resolve current executable: {e}"))?;
                        let mut cmd = Command::new(exe);
                        cmd.arg("core")
                            .arg("serve")
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
            if crate::core_rpc::ping().await {
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

    pub async fn shutdown(&self) {
        let mut child_guard = self.child.lock().await;
        if let Some(child) = child_guard.as_mut() {
            let _ = child.kill().await;
        }
        *child_guard = None;
        drop(child_guard);

        let mut task_guard = self.task.lock().await;
        if let Some(task) = task_guard.take() {
            task.abort();
            let _ = task.await;
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

pub fn default_core_run_mode(daemon_mode: bool) -> CoreRunMode {
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

    if daemon_mode {
        CoreRunMode::ChildProcess
    } else {
        CoreRunMode::InProcess
    }
}

pub fn default_core_bin() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("OPENHUMAN_CORE_BIN") {
        let candidate = PathBuf::from(path);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    // Dev ergonomics: in debug builds, prefer spawning this same executable with
    // `core serve` so Cargo recompiles core logic changes as part of tauri dev.
    // Sidecar discovery remains enabled for packaged/release builds.
    if cfg!(debug_assertions) {
        return None;
    }

    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;

    #[cfg(windows)]
    let standalone = exe_dir.join("openhuman.exe");
    #[cfg(not(windows))]
    let standalone = exe_dir.join("openhuman");

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

    // Sidecar layout: bundle.externalBin("binaries/openhuman") is emitted as
    // openhuman-<target-triple>(.exe) under app resources.
    let mut search_dirs = vec![exe_dir.to_path_buf()];
    #[cfg(target_os = "macos")]
    {
        if let Some(resources_dir) = exe_dir.parent().map(|p| p.join("Resources")) {
            search_dirs.push(resources_dir);
        }
    }

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
            let matches = (file_name.starts_with("openhuman-") && file_name.ends_with(".exe"))
                || (file_name.starts_with("openhuman-core-") && file_name.ends_with(".exe"));
            #[cfg(not(windows))]
            let matches =
                file_name.starts_with("openhuman-") || file_name.starts_with("openhuman-core-");

            if matches && !same_executable_path(&path, &exe) {
                return Some(path);
            }
        }
    }

    None
}
