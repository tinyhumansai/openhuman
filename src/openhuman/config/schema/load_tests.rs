use super::dirs::{ACTION_DIR_ENV_VAR, MEMORY_SYNC_INTERVAL_SECS_ENV_VAR};
use super::env::EnvLookup;
use super::*;
use crate::openhuman::config::schema::{StreamMode, TelegramConfig};

// ── apply_env_overrides ────────────────────────────────────────

use crate::openhuman::config::TEST_ENV_LOCK as ENV_LOCK;

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

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

async fn load_or_init_for_workspace(root: &std::path::Path) -> Config {
    let env = MapEnv::default().with("OPENHUMAN_WORKSPACE", root.to_str().unwrap());
    Config::load_or_init_with_env_lookup(root, &root.join("workspace"), &env)
        .await
        .unwrap()
}

// ── non-UTF-8 (binary) config recovery (#5167) ──────────────────────────

/// Helper: write binary (non-UTF-8) bytes to a file.
async fn write_binary(path: &std::path::Path, bytes: &[u8]) {
    tokio::fs::write(path, bytes)
        .await
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", path.display()));
}

#[path = "load_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "load_tests_part_02_tests.rs"]
mod part_02_tests;
#[path = "load_tests_part_03_tests.rs"]
mod part_03_tests;
#[path = "load_tests_part_04_tests.rs"]
mod part_04_tests;
#[path = "load_tests_part_05_tests.rs"]
mod part_05_tests;
