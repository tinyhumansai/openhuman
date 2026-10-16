use super::*;

#[test]
fn cache_key_changes_when_backend_credential_rotates() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };

    crate::security::credentials::api_key::store_api_key(&config, "tiny_first_key").unwrap();
    let first = cache_key(&config);
    assert!(!first.contains("tiny_first_key"));

    crate::security::credentials::api_key::store_api_key(&config, "tiny_second_key").unwrap();
    let second = cache_key(&config);
    assert_ne!(first, second);
    assert!(!second.contains("tiny_second_key"));
}

#[test]
fn cache_key_changes_when_backend_url_changes() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    crate::security::credentials::api_key::store_api_key(&config, "tiny_test_key").unwrap();
    let first = cache_key(&config);
    // The control-plane origin a host's config enricher resolved.
    config.resolved_endpoints.control_plane_url = Some("https://other.example".to_string());
    assert_ne!(first, cache_key(&config));
}

#[test]
fn cache_key_changes_when_resolved_fallback_url_changes() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        api_url: Some("http://localhost:11434/v1".to_string()),
        ..Config::default()
    };
    let first = cache_key_with_backend_url(&config, "https://backend-one.example");
    let second = cache_key_with_backend_url(&config, "https://backend-two.example");
    assert_ne!(first, second);
}
