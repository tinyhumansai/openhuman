//! JSON-RPC / CLI controller surface for platform service install/lifecycle.

use crate::openhuman::config::Config;
use crate::openhuman::service::daemon_host::DaemonHostConfig;
use crate::openhuman::service::{self, daemon_host, ServiceStatus};
use crate::rpc::RpcOutcome;

/// Installs the OpenHuman daemon as a system service.
pub async fn service_install(config: &Config) -> Result<RpcOutcome<ServiceStatus>, String> {
    let status = service::install(config).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(status, "service install completed"))
}

/// Starts the installed OpenHuman daemon service.
pub async fn service_start(config: &Config) -> Result<RpcOutcome<ServiceStatus>, String> {
    let status = service::start(config).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(status, "service start completed"))
}

/// Stops the running OpenHuman daemon service.
pub async fn service_stop(config: &Config) -> Result<RpcOutcome<ServiceStatus>, String> {
    let status = service::stop(config).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(status, "service stop completed"))
}

/// Returns the current status of the OpenHuman daemon service.
pub async fn service_status(config: &Config) -> Result<RpcOutcome<ServiceStatus>, String> {
    let status = service::status(config).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(status, "service status fetched"))
}

/// Requests an asynchronous restart of the core process.
pub async fn service_restart(
    source: Option<String>,
    reason: Option<String>,
) -> Result<RpcOutcome<service::RestartStatus>, String> {
    service::restart::service_restart(source, reason).await
}

/// Requests an asynchronous graceful shutdown of the core process.
pub async fn service_shutdown(
    source: Option<String>,
    reason: Option<String>,
) -> Result<RpcOutcome<service::ShutdownStatus>, String> {
    service::shutdown::service_shutdown(source, reason).await
}

/// Uninstalls the OpenHuman daemon system service.
pub async fn service_uninstall(config: &Config) -> Result<RpcOutcome<ServiceStatus>, String> {
    let status = service::uninstall(config).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        status,
        "service uninstall completed",
    ))
}

/// Reads the daemon host UI preferences from the configuration directory.
pub async fn daemon_host_get(config: &Config) -> Result<RpcOutcome<DaemonHostConfig>, String> {
    let config_dir = config
        .config_path
        .parent()
        .ok_or_else(|| "failed to resolve config directory".to_string())?;
    let current = daemon_host::load_for_config_dir(config_dir).await;
    Ok(RpcOutcome::single_log(current, "daemon host config loaded"))
}

/// Updates the daemon host UI preferences and saves them to disk.
pub async fn daemon_host_set(
    config: &Config,
    show_tray: bool,
) -> Result<RpcOutcome<DaemonHostConfig>, String> {
    let config_dir = config
        .config_path
        .parent()
        .ok_or_else(|| "failed to resolve config directory".to_string())?;
    let next = DaemonHostConfig { show_tray };
    daemon_host::save_for_config_dir(config_dir, &next).await?;
    Ok(RpcOutcome::single_log(next, "daemon host config saved"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config(tmp: &TempDir) -> Config {
        Config {
            workspace_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        }
    }

    // NOTE: `service_install`, `service_start`, `service_stop`,
    // `service_status`, `service_uninstall`, and `service_restart`
    // mutate real OS state (launchctl / systemd) or terminate the
    // process. They are not safe to exercise from unit tests; the
    // RPC adapter tests live in tests/json_rpc_e2e.rs.

    // ── daemon_host_get / set ────────────────────────────────────

    #[tokio::test]
    async fn daemon_host_get_returns_default_when_no_file_present() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        // Ensure the config dir exists so `load_for_config_dir` can
        // operate (most loaders treat a missing dir as "use default").
        std::fs::create_dir_all(tmp.path()).unwrap();
        let out = daemon_host_get(&config).await.unwrap();
        // No assertion on `show_tray` value — defaults vary by build.
        // The contract under test is that the function returns Ok with
        // the canonical log line and a deterministic struct shape.
        assert!(out
            .logs
            .iter()
            .any(|l| l.contains("daemon host config loaded")));
        let _ = out.value.show_tray;
    }

    #[tokio::test]
    async fn daemon_host_set_persists_value_visible_to_subsequent_get() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        std::fs::create_dir_all(tmp.path()).unwrap();

        // Write `show_tray = false`, then read it back.
        let saved = daemon_host_set(&config, false).await.unwrap();
        assert!(!saved.value.show_tray);
        assert!(saved
            .logs
            .iter()
            .any(|l| l.contains("daemon host config saved")));

        let loaded = daemon_host_get(&config).await.unwrap();
        assert!(
            !loaded.value.show_tray,
            "set→get round-trip must observe the persisted value"
        );

        // Flip it back and confirm the toggle round-trips too.
        let saved = daemon_host_set(&config, true).await.unwrap();
        assert!(saved.value.show_tray);
        let loaded = daemon_host_get(&config).await.unwrap();
        assert!(loaded.value.show_tray);
    }

    #[tokio::test]
    async fn daemon_host_get_errors_when_config_path_has_no_parent() {
        // A config_path of just a filename (no parent directory) trips
        // the "failed to resolve config directory" guard.
        let mut config = Config::default();
        config.config_path = std::path::PathBuf::from("");
        let err = daemon_host_get(&config).await.unwrap_err();
        assert!(
            err.contains("failed to resolve config directory"),
            "expected config-dir error, got: {err}"
        );
    }

    #[tokio::test]
    async fn daemon_host_set_errors_when_config_path_has_no_parent() {
        let mut config = Config::default();
        config.config_path = std::path::PathBuf::from("");
        let err = daemon_host_set(&config, true).await.unwrap_err();
        assert!(err.contains("failed to resolve config directory"));
    }
}
