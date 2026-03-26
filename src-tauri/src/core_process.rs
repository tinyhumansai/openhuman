use std::sync::Arc;

use tokio::process::{Child, Command};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct CoreProcessHandle {
    child: Arc<Mutex<Option<Child>>>,
    port: u16,
}

impl CoreProcessHandle {
    pub fn new(port: u16) -> Self {
        Self {
            child: Arc::new(Mutex::new(None)),
            port,
        }
    }

    pub fn rpc_url(&self) -> String {
        format!("http://127.0.0.1:{}/rpc", self.port)
    }

    pub async fn ensure_running(&self) -> Result<(), String> {
        if crate::core_rpc::ping().await {
            log::info!("[core] found existing core rpc endpoint at {}", self.rpc_url());
            return Ok(());
        }

        let mut guard = self.child.lock().await;
        if guard.is_none() {
            let exe = std::env::current_exe()
                .map_err(|e| format!("failed to resolve current executable: {e}"))?;

            let mut cmd = Command::new(exe);
            cmd.arg("core")
                .arg("serve")
                .arg("--port")
                .arg(self.port.to_string());

            log::info!("[core] spawning core process: {:?} core serve --port {}", cmd.as_std().get_program(), self.port);

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
