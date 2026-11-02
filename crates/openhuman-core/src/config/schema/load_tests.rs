use super::dirs::ACTION_DIR_ENV_VAR;
use super::env::EnvLookup;
use super::*;
use crate::config::schema::{StreamMode, TelegramConfig};

// ── apply_env_overrides ────────────────────────────────────────

use crate::config::TEST_ENV_LOCK as ENV_LOCK;

fn clear_env(keys: &[&str]) {
    for key in keys {
        unsafe {
            std::env::remove_var(key);
        }
    }
}

// ── EnvLookup seam for resolve_runtime_config_dirs ─────────────

#[derive(Default)]
struct MapEnv(std::collections::HashMap<String, String>);

impl MapEnv {
    fn with(mut self, k: &str, v: &str) -> Self {
        self.0.insert(k.to_string(), v.to_string());
        self
    }
}

impl EnvLookup for MapEnv {
    fn get(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
}

// ── apply_env_overlay_with: EnvLookup seam ─────────────────────
//
// These tests exercise every env override branch via a `HashMapEnv`
// fixture so they neither mutate the process environment nor need
// to grab `TEST_ENV_LOCK`. They can all run in parallel.

use std::collections::HashMap;

/// In-memory [`EnvLookup`] used by the overlay tests. Case-sensitive
/// to mirror Unix `std::env::var` semantics.
#[derive(Default)]
struct HashMapEnv {
    entries: HashMap<String, String>,
}

impl HashMapEnv {
    fn new() -> Self {
        Self::default()
    }

    fn with(mut self, key: &str, value: &str) -> Self {
        self.entries.insert(key.to_string(), value.to_string());
        self
    }
}

impl EnvLookup for HashMapEnv {
    fn get(&self, key: &str) -> Option<String> {
        self.entries.get(key).cloned()
    }

    fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }
}

// ── config recovery (load_or_init with corrupted config.toml) ───

/// Helper: write a file under a temp dir path.
async fn write_file(path: &std::path::Path, contents: &str) {
    tokio::fs::write(path, contents)
        .await
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", path.display()));
}

const CORRUPTED_TOML: &str = "{{{ bad table header\n";

fn env_lock() -> tokio::sync::MutexGuard<'static, ()> {
    ENV_LOCK.blocking_lock()
}

async fn load_or_init_for_workspace(root: &std::path::Path) -> Config {
    let env = MapEnv::default().with("OPENHUMAN_WORKSPACE", root.to_str().unwrap());
    Config::load_or_init_with_env_lookup(root, &root.join("workspace"), &env)
        .await
        .unwrap()
}

#[tokio::test]
async fn preflight_config_load_preserves_sealed_secrets_until_security_is_ready() {
    let root = tempfile::tempdir().unwrap();
    let config_path = root.path().join("config.toml");
    let sealed = format!("enc2:{}", "00".repeat(32));
    let config = Config {
        config_path: config_path.clone(),
        workspace_dir: root.path().join("workspace"),
        secrets: crate::config::schema::SecretsConfig { encrypt: true },
        api_key: Some(sealed.clone()),
        ..Config::default()
    };
    tokio::fs::write(&config_path, toml::to_string(&config).unwrap())
        .await
        .unwrap();
    let env = MapEnv::default();

    let loaded = Config::load_or_init_unsealed_with_env_lookup(
        root.path(),
        &root.path().join("workspace"),
        &env,
    )
    .await
    .unwrap();

    assert_eq!(loaded.api_key.as_deref(), Some(sealed.as_str()));
}

// ── non-UTF-8 (binary) config recovery (#5167) ──────────────────────────

/// Helper: write binary (non-UTF-8) bytes to a file.
async fn write_binary(path: &std::path::Path, bytes: &[u8]) {
    tokio::fs::write(path, bytes)
        .await
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", path.display()));
}

#[path = "load_active_user_and_dirs_tests.rs"]
mod active_user_and_dirs_tests;
#[path = "load_backup_tests.rs"]
mod backup_tests;
#[path = "load_components_migration_tests.rs"]
mod components_migration_tests;
#[path = "load_corruption_recovery_tests.rs"]
mod corruption_recovery_tests;
#[path = "load_env_overlay_context_tests.rs"]
mod env_overlay_context_tests;
#[path = "load_env_overlay_tests.rs"]
mod env_overlay_tests;
#[path = "load_migration_tests.rs"]
mod migration_tests;
#[path = "load_parse_log_tests.rs"]
mod parse_log_tests;
#[path = "load_profile_layout_parity_tests.rs"]
mod profile_layout_parity_tests;

#[path = "load_managed_provider_tests.rs"]
mod managed_provider_tests;
