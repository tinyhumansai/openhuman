//! Test fixtures shared by the hosted domains' `*_tests.rs`.

use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::credentials::{AuthService, APP_SESSION_PROVIDER};
use tempfile::TempDir;

/// A config rooted in `tmp` whose backend is `api_url`.
pub fn config(tmp: &TempDir, api_url: &str) -> Config {
    crate::install(crate::InstallOptions::default()).expect("install mock backend transport");
    let mut config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        api_url: Some(api_url.to_string()),
        ..Config::default()
    };
    // These fixtures store credentials in disposable test workspaces. The
    // encrypted config store delegates cipher operations to the loaded
    // TinySecurity module, which these focused hosted-client tests do not boot.
    config.secrets.encrypt = false;
    config
}

/// Store `token` as the app-session credential for `config`.
pub async fn store_session(config: &Config, token: &str) {
    // These hosted adapter tests do not boot TinySecurity; `config()` disables
    // config-secret encryption for their disposable workspace, so storing a
    // session token needs no module-owned cipher scope.
    AuthService::from_config(config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            "default",
            token,
            std::collections::HashMap::new(),
            true,
        )
        .await
        .expect("store session token");
}

/// A signed-in config against `api_url` (session token `jwt.test`).
pub async fn signed_in(tmp: &TempDir, api_url: &str) -> Config {
    crate::install(
        crate::InstallOptions::default()
            .hosted_controllers(false)
            .tool_ranker(false),
    )
    .expect("install SDK backend transport for hosted mock");
    let config = config(tmp, api_url);
    store_session(&config, "jwt.test").await;
    config
}

/// A config signed in with the offline local credential ("Continue locally").
pub async fn local_session(tmp: &TempDir) -> Config {
    // Unroutable: any request would fail loudly instead of returning the sentinel.
    let config = config(tmp, "http://127.0.0.1:9");
    store_session(&config, "desktop.test.local").await;
    config
}
