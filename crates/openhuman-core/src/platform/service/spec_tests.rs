//! Tests for the core daemon's service description and its dispatch.

use super::*;
use openhuman_platform::service::ServiceState;
use tempfile::TempDir;

fn test_config(tmp: &TempDir) -> Config {
    Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    }
}

#[test]
fn spec_runs_the_daemon_in_the_config_dir() {
    let tmp = TempDir::new().unwrap();
    let spec = service_spec(&test_config(&tmp)).expect("spec");

    assert_eq!(spec.work_dir, tmp.path());
    assert_eq!(spec.logs_dir, tmp.path().join("logs"));
    assert_eq!(spec.args, vec!["run".to_string()]);
}

#[test]
fn spec_keeps_the_published_service_labels() {
    let tmp = TempDir::new().unwrap();
    let spec = service_spec(&test_config(&tmp)).expect("spec");

    #[cfg(target_os = "macos")]
    {
        assert_eq!(spec.label, "com.openhuman.core");
        assert_eq!(
            spec.legacy_labels,
            vec![
                "com.openhuman.daemon".to_string(),
                "com.openhuman.app".to_string()
            ]
        );
    }
    #[cfg(target_os = "linux")]
    {
        assert_eq!(spec.label, "openhuman.service");
        assert!(spec.legacy_labels.is_empty());
    }
    #[cfg(windows)]
    {
        assert_eq!(spec.label, "OpenHuman Core");
        assert!(spec.legacy_labels.is_empty());
    }
}

#[test]
fn spec_prefers_the_core_bin_override() {
    let tmp = TempDir::new().unwrap();
    let bin = tmp.path().join("openhuman-core-override");
    std::fs::write(&bin, b"").unwrap();
    let _env = crate::config::test_env::EnvVarGuard::locked_set("OPENHUMAN_CORE_BIN", &bin);

    let spec = service_spec(&test_config(&tmp)).expect("spec");
    assert_eq!(spec.exe, bin);
}

#[test]
fn dispatch_routes_install_and_status_to_the_mock_when_env_set() {
    let tmp = TempDir::new().unwrap();
    let state_file = tmp.path().join("svc-mock-state.json");
    let _env = crate::config::test_env::EnvVarGuard::locked()
        .with("OPENHUMAN_SERVICE_MOCK", "1")
        .with("OPENHUMAN_SERVICE_MOCK_STATE_FILE", &state_file);
    let cfg = test_config(&tmp);

    let fresh = status(&cfg).expect("status via mock");
    assert!(
        matches!(fresh.state, ServiceState::NotInstalled),
        "fresh mock state must be NotInstalled, got {:?}",
        fresh.state
    );

    let installed = install(&cfg).expect("install via mock");
    assert!(
        matches!(installed.state, ServiceState::Stopped),
        "dispatch → mock install must yield Stopped, got {:?}",
        installed.state
    );
    assert_eq!(installed.details.as_deref(), Some("service mock backend"));
    assert_eq!(installed.label, service_spec(&cfg).unwrap().label);
}
