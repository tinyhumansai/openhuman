use super::*;
use tempfile::tempdir;

// ── env_flag_enabled ────────────────────────────────────────────

use crate::config::TEST_ENV_LOCK as ENV_LOCK;

#[tokio::test]
async fn sandbox_settings_reject_unpersisted_env_passthrough_updates() {
    let tmp = tempdir().unwrap();
    let mut config = tmp_config(&tmp);
    let result = crate::config::ops::apply_sandbox_settings(
        &mut config,
        crate::config::ops::SandboxSettingsPatch {
            env_passthrough: Some(vec!["CUSTOM_VAR".into()]),
            ..Default::default()
        },
    )
    .await;

    assert!(result.unwrap_err().contains("env_passthrough"));
}

#[tokio::test]
async fn sandbox_settings_reject_retired_firejail_and_bubblewrap_backends() {
    let tmp = tempdir().unwrap();
    let config = tmp_config(&tmp);

    for backend in ["firejail", "bubblewrap"] {
        let mut config = config.clone();
        let error = crate::config::ops::apply_sandbox_settings(
            &mut config,
            crate::config::ops::SandboxSettingsPatch {
                backend: Some(backend.to_owned()),
                ..Default::default()
            },
        )
        .await
        .expect_err("retired backend must not be saved");

        assert!(error.contains("TinyBox"), "{error}");
    }
}

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
#[path = "ops_voice_and_autonomy_tests.rs"]
mod voice_and_autonomy_tests;
