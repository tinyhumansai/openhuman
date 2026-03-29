//! Internal daemon supervisor hook for the desktop host.
//!
//! Full supervisor logic can be restored to spawn/monitor the core process; for now this
//! waits until the app signals shutdown via `CancellationToken`.

use anyhow::Result;
use openhuman_core::DaemonConfig;
use tauri::AppHandle;
use tokio_util::sync::CancellationToken;

pub struct DaemonHandle {
    pub cancel: CancellationToken,
}

pub async fn run(_config: DaemonConfig, _app: AppHandle, cancel: CancellationToken) -> Result<()> {
    log::info!("[openhuman_daemon] supervisor idle until shutdown (stub build)");
    cancel.cancelled().await;
    Ok(())
}
