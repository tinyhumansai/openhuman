use super::*;
use tempfile::tempdir;

// ── env_flag_enabled ────────────────────────────────────────────

use crate::openhuman::config::TEST_ENV_LOCK as ENV_LOCK;

// ── apply_*_settings ─────────────────────────────────────────

fn tmp_config(tmp: &tempfile::TempDir) -> Config {
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().join("workspace");
    cfg.config_path = tmp.path().join("config.toml");
    std::fs::create_dir_all(&cfg.workspace_dir).unwrap();
    cfg
}

#[path = "ops_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "ops_tests_part_02_tests.rs"]
mod part_02_tests;
#[path = "ops_tests_part_03_tests.rs"]
mod part_03_tests;
#[path = "ops_tests_part_04_tests.rs"]
mod part_04_tests;
