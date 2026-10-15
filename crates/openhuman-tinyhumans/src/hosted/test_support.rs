//! Test fixtures shared by the hosted domains' `*_tests.rs`.

use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::security::credentials::{AuthService, APP_SESSION_PROVIDER};
use tempfile::TempDir;

/// A config rooted in `tmp` whose backend is `api_url`.
pub fn config(tmp: &TempDir, api_url: &str) -> Config {
    init_keyring();
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
    init_keyring();
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
    init_keyring();
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

/// Initialize the encrypted keyring before core caches a missing OS keychain.
pub fn init_keyring() {
    static KEYRING: std::sync::OnceLock<TempDir> = std::sync::OnceLock::new();
    KEYRING.get_or_init(|| {
        use aes_gcm::aead::rand_core::RngCore;
        let directory = tempfile::tempdir().expect("scratch keyring workspace");
        let mut bytes = [0_u8; 32];
        aes_gcm::aead::OsRng.fill_bytes(&mut bytes);
        let key: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        std::env::set_var("OPENHUMAN_WORKSPACE", directory.path());
        std::env::set_var("OPENHUMAN_KEYRING_BACKEND", "encrypted_file");
        std::env::remove_var("OPENHUMAN_KEYRING_MASTER_KEY_FILE");
        std::env::set_var("OPENHUMAN_KEYRING_MASTER_KEY", key);
        openhuman_embed::process::init_master_key().expect("headless test master key");
        directory
    });
}
