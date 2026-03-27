use std::path::PathBuf;
use std::sync::Arc;

use tokio::process::{Child, Command};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct CoreProcessHandle {
    child: Arc<Mutex<Option<Child>>>,
    port: u16,
    core_bin: Option<PathBuf>,
}

impl CoreProcessHandle {
    pub fn new(port: u16, core_bin: Option<PathBuf>) -> Self {
        Self {
            child: Arc::new(Mutex::new(None)),
            port,
            core_bin,
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

        let mut guard = self.child.lock().await;
        if guard.is_none() {
            let mut cmd = if let Some(core_bin) = &self.core_bin {
                let mut cmd = Command::new(core_bin);
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
        drop(guard);

        for _ in 0..40 {
            if crate::core_rpc::ping().await {
                log::info!("[core] core rpc became ready at {}", self.rpc_url());
                return Ok(());
            }

            let mut guard = self.child.lock().await;
            if let Some(child) = guard.as_mut() {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        return Err(format!("core process exited before ready: {status}"));
                    }
                    Ok(None) => {}
                    Err(e) => return Err(format!("failed checking core process status: {e}")),
                }
            }
            drop(guard);
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        Err("core process did not become ready".to_string())
    }

    pub async fn shutdown(&self) {
        let mut guard = self.child.lock().await;
        if let Some(child) = guard.as_mut() {
            let _ = child.kill().await;
        }
        *guard = None;
    }
}

pub fn default_core_port() -> u16 {
    std::env::var("OPENHUMAN_CORE_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(7788)
}

pub fn default_core_bin() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("OPENHUMAN_CORE_BIN") {
        let candidate = PathBuf::from(path);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;

    #[cfg(windows)]
    let standalone = exe_dir.join("openhuman-core.exe");
    #[cfg(not(windows))]
    let standalone = exe_dir.join("openhuman-core");

    if standalone.exists() {
        return Some(standalone);
    }

    // Sidecar layout: bundle.externalBin("binaries/openhuman-core") is emitted as
    // openhuman-core-<target-triple>(.exe) under app resources.
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
            let matches = file_name.starts_with("openhuman-core-") && file_name.ends_with(".exe");
            #[cfg(not(windows))]
            let matches = file_name.starts_with("openhuman-core-");

            if matches {
                return Some(path);
            }
        }
    }

    None
}
