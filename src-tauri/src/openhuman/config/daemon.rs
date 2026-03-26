//! Tauri-focused daemon configuration wrapper for openhuman.

use crate::openhuman::config::{
    AuditConfig, AutonomyConfig, ReliabilityConfig, SecretsConfig, SecurityConfig,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Top-level daemon configuration for the Tauri supervisor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    /// Root data directory (defaults to Tauri's `app_data_dir/openhuman`).
    pub data_dir: PathBuf,
    /// Workspace directory the agent may operate within.
    pub workspace_dir: PathBuf,
    /// Autonomy / command-policy settings.
    #[serde(default)]
    pub autonomy: AutonomyConfig,
    /// Security / sandbox settings.
    #[serde(default)]
    pub security: SecurityConfig,
    /// Reliability / backoff settings.
    #[serde(default)]
    pub reliability: ReliabilityConfig,
    /// Encrypted secret store settings.
    #[serde(default)]
    pub secrets: SecretsConfig,
    /// Audit logging settings.
    #[serde(default)]
    pub audit: AuditConfig,
}

impl DaemonConfig {
    /// Build a config that derives paths from the Tauri `app_data_dir`.
    pub fn from_app_data_dir(app_data_dir: &std::path::Path) -> Self {
        let data_dir = app_data_dir.join("openhuman");
        let workspace_dir = data_dir.join("workspace");
        log::info!(
            "[openhuman:config] Initialized config: data_dir={}, workspace_dir={}",
            data_dir.display(),
            workspace_dir.display()
        );
        Self {
            data_dir,
            workspace_dir,
            autonomy: AutonomyConfig::default(),
            security: SecurityConfig::default(),
            reliability: ReliabilityConfig::default(),
            secrets: SecretsConfig::default(),
            audit: AuditConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_config_from_app_data_dir() {
        let app_data = std::path::PathBuf::from("/tmp/test-openhuman");
        let config = DaemonConfig::from_app_data_dir(&app_data);

        assert_eq!(config.data_dir, app_data.join("openhuman"));
        assert_eq!(
            config.workspace_dir,
            app_data.join("openhuman").join("workspace")
        );
    }
}
