//! Test fixtures shared by the hosted domains' `*_tests.rs`.

use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::security::credentials::{AuthService, APP_SESSION_PROVIDER};
use tempfile::TempDir;

/// A config rooted in `tmp` whose backend is `api_url`.
pub fn config(tmp: &TempDir, api_url: &str) -> Config {
    crate::install(crate::InstallOptions::default()).expect("install mock backend transport");
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
    // The core is a dependency here, so its cfg(test) plaintext keyring is
    // unavailable. Boot the encrypted store with an isolated fixture key just
    // as a headless host does, before the first credential operation.
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let workspace = TempDir::new().expect("fixture keyring workspace");
        let variables = [
            "OPENHUMAN_KEYRING_MASTER_KEY",
            "OPENHUMAN_KEYRING_MASTER_KEY_FILE",
            "OPENHUMAN_KEYRING_BACKEND",
            "OPENHUMAN_WORKSPACE",
        ];
        let previous = variables.map(|name| (name, std::env::var_os(name)));
        std::env::remove_var("OPENHUMAN_KEYRING_MASTER_KEY_FILE");
        std::env::set_var("OPENHUMAN_KEYRING_MASTER_KEY", "11".repeat(32));
        std::env::set_var("OPENHUMAN_KEYRING_BACKEND", "encrypted_file");
        std::env::set_var("OPENHUMAN_WORKSPACE", workspace.path());
        let initialized = openhuman_embed::process::init_master_key();
        for (name, value) in previous {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
        initialized.expect("initialize fixture master key");
    });
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
    crate::install(
        crate::InstallOptions::default()
            .hosted_controllers(false)
            .tool_ranker(false),
    )
    .expect("install SDK backend transport for hosted mock");
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
