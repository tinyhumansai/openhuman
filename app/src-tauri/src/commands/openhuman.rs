//! Local daemon-host Tauri commands.
//!
//! OpenHuman core JSON-RPC calls are routed through `core_rpc_relay`.
//! This module only keeps machine-local host config commands that are not part
//! of core RPC.

use openhuman_core::core_server::CommandResponse;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonHostConfigPayload {
    pub show_tray: bool,
}

/// Read local daemon-host settings (machine-local, not synced through core RPC).
#[tauri::command]
pub async fn openhuman_get_daemon_host_config(
    app: tauri::AppHandle,
) -> Result<CommandResponse<DaemonHostConfigPayload>, String> {
    let cfg = crate::daemon_host_config::load(&app).await;
    Ok(CommandResponse {
        result: DaemonHostConfigPayload {
            show_tray: cfg.show_tray,
        },
        logs: vec!["daemon host config loaded".to_string()],
    })
}

/// Update local daemon-host settings (machine-local, not synced through core RPC).
#[tauri::command]
pub async fn openhuman_set_daemon_host_config(
    app: tauri::AppHandle,
    show_tray: Option<bool>,
) -> Result<CommandResponse<DaemonHostConfigPayload>, String> {
    let mut cfg = crate::daemon_host_config::load(&app).await;
    if let Some(value) = show_tray {
        cfg.show_tray = value;
    }

    crate::daemon_host_config::save(&app, &cfg).await?;

    Ok(CommandResponse {
        result: DaemonHostConfigPayload {
            show_tray: cfg.show_tray,
        },
        logs: vec![
            "daemon host config saved".to_string(),
            "restart daemon host process to apply tray visibility changes".to_string(),
        ],
    })
}
