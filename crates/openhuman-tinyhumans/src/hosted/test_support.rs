//! Test fixtures shared by the hosted domains' `*_tests.rs`.

use openhuman_core::config::Config;
use openhuman_core::security::credentials::{AuthService, APP_SESSION_PROVIDER};
use tempfile::TempDir;

/// A config rooted in `tmp` whose backend is `api_url`.
pub fn config(tmp: &TempDir, api_url: &str) -> Config {
    Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        api_url: Some(api_url.to_string()),
        ..Config::default()
    }
}

/// Store `token` as the app-session credential for `config`.
pub fn store_session(config: &Config, token: &str) {
    AuthService::from_config(config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            "default",
            token,
            std::collections::HashMap::new(),
            true,
        )
        .expect("store session token");
}

/// A signed-in config against `api_url` (session token `jwt.test`).
pub fn signed_in(tmp: &TempDir, api_url: &str) -> Config {
    let config = config(tmp, api_url);
    store_session(&config, "jwt.test");
    config
}

/// A config signed in with the offline local credential ("Continue locally").
pub fn local_session(tmp: &TempDir) -> Config {
    // Unroutable: any request would fail loudly instead of returning the sentinel.
    let config = config(tmp, "http://127.0.0.1:9");
    store_session(&config, "desktop.test.local");
    config
}
