use super::*;
use crate::openhuman::channels::email_channel::EmailConfig;
use crate::openhuman::security::credentials::AuthService;
use std::collections::HashMap;
use tempfile::tempdir;

fn isolated_config() -> (tempfile::TempDir, Config) {
    let tmp = tempdir().expect("tempdir");
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace dir");
    (tmp, config)
}

fn store_email_creds(config: &Config, username: &str, password: &str) {
    let auth = AuthService::from_config(config);
    let mut metadata = HashMap::new();
    metadata.insert("username".to_string(), username.to_string());
    metadata.insert("password".to_string(), password.to_string());
    auth.store_provider_token("channel:email:api_key", "default", "", metadata, true)
        .expect("store credentials");
}

#[test]
fn loads_password_from_credentials_when_toml_empty() {
    let (_tmp, config) = isolated_config();
    store_email_creds(&config, "me@example.com", "from-credentials");

    let cfg = EmailConfig {
        username: "me@example.com".into(),
        password: String::new(),
        ..EmailConfig::default()
    };
    let resolved = resolve_email_password(cfg, &config);
    assert_eq!(resolved.password, "from-credentials");
}

#[test]
fn preserves_existing_toml_password_without_consulting_store() {
    let (_tmp, config) = isolated_config();
    let cfg = EmailConfig {
        username: "me@example.com".into(),
        password: "from-toml".into(),
        ..EmailConfig::default()
    };
    let resolved = resolve_email_password(cfg, &config);
    assert_eq!(resolved.password, "from-toml");
}

#[test]
fn skips_hydration_when_stored_profile_has_different_username() {
    // User changed `username` in config.toml; the stored profile is for the
    // old account. The resolver must not graft the old password onto it.
    let (_tmp, config) = isolated_config();
    store_email_creds(&config, "old@example.com", "old-password-do-not-use");

    let cfg = EmailConfig {
        username: "new@example.com".into(),
        password: String::new(),
        ..EmailConfig::default()
    };
    let resolved = resolve_email_password(cfg, &config);
    assert_eq!(
        resolved.password, "",
        "stale profile for old username must not hydrate the new account",
    );
}
