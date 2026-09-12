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
