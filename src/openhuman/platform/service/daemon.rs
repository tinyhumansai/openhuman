use crate::openhuman::config::Config;
use std::path::PathBuf;

/// Shared daemon state file path used by health/doctor reporting.
pub fn state_file_path(config: &Config) -> PathBuf {
    config
        .config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join("daemon_state.json")
}
