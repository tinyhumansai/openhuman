//! Machine-local daemon UI preferences (tray visibility, etc.).
//! Stored next to the main OpenHuman config file.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

fn config_file_path(openhuman_base: &Path) -> PathBuf {
    openhuman_base.join("daemon_host_config.json")
}

pub async fn load_for_config_dir(openhuman_base: &Path) -> DaemonHostConfig {
    let path = config_file_path(openhuman_base);
    let Ok(contents) = tokio::fs::read_to_string(path).await else {
        return DaemonHostConfig::default();
    };
    serde_json::from_str::<DaemonHostConfig>(&contents).unwrap_or_default()
}

pub async fn save_for_config_dir(
    openhuman_base: &Path,
    config: &DaemonHostConfig,
) -> Result<(), String> {
    let path = config_file_path(openhuman_base);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("failed to create daemon host config directory: {e}"))?;
    }
    let bytes = serde_json::to_vec_pretty(config)
        .map_err(|e| format!("failed to serialize daemon host config: {e}"))?;
    tokio::fs::write(path, bytes)
        .await
        .map_err(|e| format!("failed to write daemon host config: {e}"))
}
