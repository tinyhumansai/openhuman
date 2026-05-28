use tempfile::TempDir;

use super::*;
use crate::openhuman::config::Config;

fn test_config(tmp: &TempDir) -> Config {
    Config {
        workspace_dir: tmp.path().to_path_buf(),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    }
}

// ── ai_list_artifacts ──────────────────────────────────────────────────────

#[tokio::test]
async fn list_empty() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let outcome = ai_list_artifacts(&config, None, None).await.unwrap();
    let value = outcome.into_cli_compatible_json().unwrap();
    assert_eq!(value["total"], 0);
    assert_eq!(value["artifacts"].as_array().unwrap().len(), 0);
    assert_eq!(value["offset"], 0);
    assert_eq!(value["limit"], DEFAULT_LIMIT as u64);
}

// ── ai_get_artifact ────────────────────────────────────────────────────────

#[tokio::test]
async fn get_missing_id_error() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = ai_get_artifact(&config, "").await.unwrap_err();
    assert!(err.contains("must not be empty"), "unexpected error: {err}");
}

// ── ai_delete_artifact ─────────────────────────────────────────────────────

#[tokio::test]
async fn delete_missing_id_error() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = ai_delete_artifact(&config, "").await.unwrap_err();
    assert!(err.contains("must not be empty"), "unexpected error: {err}");
}
