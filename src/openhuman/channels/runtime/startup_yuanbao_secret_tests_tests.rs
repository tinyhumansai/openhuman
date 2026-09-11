use super::*;
use crate::openhuman::channels::providers::yuanbao::YuanbaoConfig;
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

#[test]
fn loads_app_secret_from_credentials_when_toml_empty() {
    let (_tmp, config) = isolated_config();
    // Pre-write the credentials the same way `connect_channel` does:
    // metadata under the `channel:yuanbao:api_key` provider key.
    let auth = AuthService::from_config(&config);
    let mut metadata = HashMap::new();
    metadata.insert("app_key".to_string(), "ak".to_string());
    metadata.insert("app_secret".to_string(), "from-credentials".to_string());
    auth.store_provider_token("channel:yuanbao:api_key", "default", "", metadata, true)
        .expect("store credentials");

    let yb = YuanbaoConfig {
        app_key: "ak".into(),
        app_secret: String::new(),
        ..Default::default()
    };
    let resolved = resolve_yuanbao_app_secret(yb, &config);
    assert_eq!(resolved.app_secret, "from-credentials");
}

#[test]
fn preserves_existing_toml_secret_without_consulting_store() {
    // No credentials in the store at all — resolver must still leave
    // the TOML-supplied secret untouched.
    let (_tmp, config) = isolated_config();
    let yb = YuanbaoConfig {
        app_key: "ak".into(),
        app_secret: "from-toml".into(),
        ..Default::default()
    };
    let resolved = resolve_yuanbao_app_secret(yb, &config);
    assert_eq!(resolved.app_secret, "from-toml");
}

#[test]
fn returns_empty_secret_when_neither_toml_nor_credentials_have_one() {
    let (_tmp, config) = isolated_config();
    let yb = YuanbaoConfig {
        app_key: "ak".into(),
        app_secret: String::new(),
        ..Default::default()
    };
    let resolved = resolve_yuanbao_app_secret(yb, &config);
    // Surfaces empty so the downstream `YuanbaoChannel::new` validate()
    // step can fail clearly, instead of attempting auth with a stale value.
    assert_eq!(resolved.app_secret, "");
}

#[test]
fn skips_hydration_when_stored_profile_has_different_app_key() {
    // Reproduces the stale-secret hazard: user changed `app_key` in
    // `config.toml` (e.g. swapped to a different bot) but the
    // credentials store still has the old key's profile. The resolver
    // must NOT graft the old secret onto the new key.
    let (_tmp, config) = isolated_config();
    let auth = AuthService::from_config(&config);
    let mut metadata = HashMap::new();
    metadata.insert("app_key".to_string(), "OLD-KEY".to_string());
    metadata.insert(
        "app_secret".to_string(),
        "old-key-secret-do-not-use".to_string(),
    );
    auth.store_provider_token("channel:yuanbao:api_key", "default", "", metadata, true)
        .expect("store credentials");

    let yb = YuanbaoConfig {
        app_key: "NEW-KEY".into(),
        app_secret: String::new(),
        ..Default::default()
    };
    let resolved = resolve_yuanbao_app_secret(yb, &config);
    assert_eq!(
        resolved.app_secret, "",
        "stale profile keyed to OLD-KEY must not hydrate NEW-KEY's secret",
    );
}
