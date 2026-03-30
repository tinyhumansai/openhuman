use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::AppHandle;
use tauri::Manager;

const DAEMON_HOST_CONFIG_FILE: &str = "daemon_host_config.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonHostConfig {
    pub show_tray: bool,
}

impl Default for DaemonHostConfig {
    fn default() -> Self {
        Self { show_tray: true }
    }
}

fn daemon_host_config_path(app: &AppHandle) -> PathBuf {
    if let Ok(app_data_dir) = app.path().app_data_dir() {
        return app_data_dir.join(DAEMON_HOST_CONFIG_FILE);
    }

    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".openhuman").join(DAEMON_HOST_CONFIG_FILE)
}

fn load_daemon_host_config(app: &AppHandle) -> DaemonHostConfig {
    let path = daemon_host_config_path(app);
    let Ok(contents) = std::fs::read_to_string(path) else {
        return DaemonHostConfig::default();
    };
    serde_json::from_str::<DaemonHostConfig>(&contents).unwrap_or_default()
}

fn save_daemon_host_config(app: &AppHandle, config: &DaemonHostConfig) -> Result<(), String> {
    let path = daemon_host_config_path(app);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create daemon host config directory: {e}"))?;
    }

    let bytes = serde_json::to_vec_pretty(config)
        .map_err(|e| format!("failed to serialize daemon host config: {e}"))?;
    std::fs::write(path, bytes).map_err(|e| format!("failed to write daemon host config: {e}"))
}

#[tauri::command]
pub async fn openhuman_get_daemon_host_config(app: AppHandle) -> Result<DaemonHostConfig, String> {
    Ok(load_daemon_host_config(&app))
}

#[tauri::command]
pub async fn openhuman_set_daemon_host_config(
    app: AppHandle,
    show_tray: bool,
) -> Result<DaemonHostConfig, String> {
    let mut cfg = load_daemon_host_config(&app);
    cfg.show_tray = show_tray;
    save_daemon_host_config(&app, &cfg)?;
    Ok(cfg)
}
