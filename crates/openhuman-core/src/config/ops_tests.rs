use super::*;
use tempfile::tempdir;

// ── env_flag_enabled ────────────────────────────────────────────

use crate::config::TEST_ENV_LOCK as ENV_LOCK;

// ── apply_*_settings ─────────────────────────────────────────

fn tmp_config(tmp: &tempfile::TempDir) -> Config {
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().join("workspace");
    cfg.config_path = tmp.path().join("config.toml");
    std::fs::create_dir_all(&cfg.workspace_dir).unwrap();
    cfg
}

#[path = "ops_agent_paths_tests.rs"]
mod agent_paths_tests;
#[path = "ops_files_dir_tests.rs"]
mod files_dir_tests;
#[path = "ops_loader_and_search_tests.rs"]
mod loader_and_search_tests;
#[path = "ops_model_and_local_ai_tests.rs"]
mod model_and_local_ai_tests;
#[path = "ops_provider_ca_tests.rs"]
mod provider_ca_tests;
#[path = "ops_provider_headers_tests.rs"]
mod provider_headers_tests;
#[path = "ops_voice_and_autonomy_tests.rs"]
mod voice_and_autonomy_tests;
