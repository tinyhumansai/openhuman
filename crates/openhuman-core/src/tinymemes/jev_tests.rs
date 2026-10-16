use super::*;

fn config_in(dir: &std::path::Path) -> Config {
    let mut config = Config::default();
    config.config_path = dir.join("config.toml");
    config.workspace_dir = dir.join("workspace");
    config.secrets.encrypt = false;
    config
}

#[test]
fn without_a_backend_credential_managed_jev_is_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    assert!(managed(&config_in(dir.path())).is_none());
}

#[test]
fn an_api_key_reaches_managed_jev() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    config.resolved_endpoints.control_plane_url = Some("https://api.example.test".into());
    crate::security::credentials::api_key::store_api_key(&config, "th_live_test").unwrap();
    assert!(managed(&config).is_some());
}
