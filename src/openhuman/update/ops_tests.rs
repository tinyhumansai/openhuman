//! Unit tests for `update::ops` — `update_version`, helper result builders,
//! and the URL / asset-name validation functions.
//!
//! These tests complement the inline `#[cfg(test)] mod tests` block in
//! `ops.rs` and focus on coverage gaps in the public helper contracts:
//! - `update_version` returns the packaged version, target triple, and
//!   asset prefix without making any network call.
//! - `already_current_result`, `missing_asset_result`, `apply_failure_result`
//!   build the correct `UpdateRunResult` shape.
//! - `build_run_result_from_staged_update` sets `restart_requested = false`
//!   under the `Supervisor` strategy.

use super::*;
use crate::openhuman::config::UpdateRestartStrategy;
use crate::openhuman::update::types::{UpdateApplyResult, UpdateInfo};

// ── update_version ────────────────────────────────────────────────────────────

#[tokio::test]
async fn update_version_returns_cargo_pkg_version() {
    let outcome = update_version().await;
    let v = outcome
        .value
        .get("version")
        .and_then(|v| v.as_str())
        .expect("version field");
    // The CARGO_PKG_VERSION env var is always non-empty for a built crate.
    assert!(!v.is_empty(), "version must be non-empty");
    // Must match the compile-time constant.
    assert_eq!(v, crate::openhuman::update::current_version());
}

#[tokio::test]
async fn update_version_returns_target_triple() {
    let outcome = update_version().await;
    let triple = outcome
        .value
        .get("target_triple")
        .and_then(|v| v.as_str())
        .expect("target_triple field");
    // Must be non-empty and match the compile-time constant.
    assert!(!triple.is_empty(), "target_triple must be non-empty");
    assert_eq!(triple, crate::openhuman::update::platform_triple());
}

#[tokio::test]
async fn update_version_asset_prefix_contains_target_triple() {
    let outcome = update_version().await;
    let triple = crate::openhuman::update::platform_triple();
    let prefix = outcome
        .value
        .get("asset_prefix")
        .and_then(|v| v.as_str())
        .expect("asset_prefix field");
    assert!(
        prefix.starts_with("openhuman-core-"),
        "asset_prefix must start with 'openhuman-core-', got: {prefix}"
    );
    assert!(
        prefix.contains(triple),
        "asset_prefix must contain target triple '{triple}', got: {prefix}"
    );
}

#[tokio::test]
async fn update_version_has_log_line() {
    let outcome = update_version().await;
    assert!(
        outcome.logs.iter().any(|l| l.contains("update_version")),
        "expected log line containing 'update_version', got: {:?}",
        outcome.logs
    );
}

// ── already_current_result ───────────────────────────────────────────────────

#[test]
fn already_current_result_no_update_and_not_applied() {
    let info = sample_update_info("0.50.0", "0.50.0", false);
    let result = already_current_result(&info, UpdateRestartStrategy::SelfReplace);
    assert!(!result.update_available);
    assert!(!result.applied);
    assert!(!result.restart_requested);
    assert!(result.staged_path.is_none());
}

#[test]
fn already_current_result_message_contains_version() {
    let info = sample_update_info("0.50.0", "0.50.0", false);
    let result = already_current_result(&info, UpdateRestartStrategy::SelfReplace);
    assert!(
        result.message.contains("0.50.0"),
        "message should contain current version, got: {}",
        result.message
    );
}

#[test]
fn already_current_result_preserves_restart_strategy_self_replace() {
    let info = sample_update_info("0.50.0", "0.50.0", false);
    let result = already_current_result(&info, UpdateRestartStrategy::SelfReplace);
    assert_eq!(result.restart_strategy, UpdateRestartStrategy::SelfReplace);
}

#[test]
fn already_current_result_preserves_restart_strategy_supervisor() {
    let info = sample_update_info("0.50.0", "0.50.0", false);
    let result = already_current_result(&info, UpdateRestartStrategy::Supervisor);
    assert_eq!(result.restart_strategy, UpdateRestartStrategy::Supervisor);
}

// ── missing_asset_result ─────────────────────────────────────────────────────

#[test]
fn missing_asset_result_update_available_but_not_applied() {
    let info = sample_update_info("0.51.0", "0.50.0", true);
    let result = missing_asset_result(info, UpdateRestartStrategy::SelfReplace);
    assert!(result.update_available);
    assert!(!result.applied);
    assert!(!result.restart_requested);
    assert!(result.staged_path.is_none());
}

#[test]
fn missing_asset_result_message_mentions_target() {
    let info = sample_update_info("0.51.0", "0.50.0", true);
    let result = missing_asset_result(info, UpdateRestartStrategy::SelfReplace);
    // The message must mention something about the platform asset being absent.
    assert!(
        result.message.contains("no asset") || result.message.contains("asset"),
        "missing-asset message should mention asset, got: {}",
        result.message
    );
}

// ── apply_failure_result ─────────────────────────────────────────────────────

#[test]
fn apply_failure_result_not_applied_and_no_staged_path() {
    let info = sample_update_info("0.51.0", "0.50.0", true);
    let result = apply_failure_result(info, UpdateRestartStrategy::SelfReplace, "timeout");
    assert!(result.update_available);
    assert!(!result.applied);
    assert!(!result.restart_requested);
    assert!(result.staged_path.is_none());
}

#[test]
fn apply_failure_result_message_contains_error() {
    let info = sample_update_info("0.51.0", "0.50.0", true);
    let result = apply_failure_result(info, UpdateRestartStrategy::SelfReplace, "network timeout");
    assert!(
        result.message.contains("network timeout"),
        "message should contain the error string, got: {}",
        result.message
    );
}

// ── build_run_result_from_staged_update (supervisor path) ────────────────────

#[tokio::test]
async fn supervisor_strategy_sets_restart_requested_false() {
    let info = sample_update_info("0.52.0", "0.51.0", true);
    let applied = UpdateApplyResult {
        installed_version: "0.52.0".to_owned(),
        staged_path: "/tmp/openhuman-core-test".to_owned(),
        restart_required: true,
        restart_strategy: UpdateRestartStrategy::SelfReplace,
    };
    let result =
        build_run_result_from_staged_update(info, applied, UpdateRestartStrategy::Supervisor).await;
    assert!(result.applied, "staged update → applied must be true");
    assert!(
        !result.restart_requested,
        "supervisor strategy must NOT set restart_requested"
    );
    assert_eq!(result.restart_strategy, UpdateRestartStrategy::Supervisor);
}

#[tokio::test]
async fn supervisor_strategy_staged_path_preserved() {
    let info = sample_update_info("0.52.0", "0.51.0", true);
    let applied = UpdateApplyResult {
        installed_version: "0.52.0".to_owned(),
        staged_path: "/tmp/openhuman-core-x86_64".to_owned(),
        restart_required: true,
        restart_strategy: UpdateRestartStrategy::SelfReplace,
    };
    let result =
        build_run_result_from_staged_update(info, applied, UpdateRestartStrategy::Supervisor).await;
    assert_eq!(
        result.staged_path.as_deref(),
        Some("/tmp/openhuman-core-x86_64"),
        "staged path must be forwarded"
    );
}

#[tokio::test]
async fn supervisor_strategy_message_mentions_supervisor() {
    let info = sample_update_info("0.52.0", "0.51.0", true);
    let applied = UpdateApplyResult {
        installed_version: "0.52.0".to_owned(),
        staged_path: "/tmp/openhuman-core".to_owned(),
        restart_required: true,
        restart_strategy: UpdateRestartStrategy::SelfReplace,
    };
    let result =
        build_run_result_from_staged_update(info, applied, UpdateRestartStrategy::Supervisor).await;
    assert!(
        result.message.contains("supervisor"),
        "supervisor strategy message should mention supervisor, got: {}",
        result.message
    );
}

// ── enforce_update_mutation_policy (via update_run) ──────────────────────────

#[tokio::test]
async fn update_run_rejected_when_rpc_mutations_disabled() {
    use crate::openhuman::config::TEST_ENV_LOCK;
    let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::TempDir::new().unwrap();

    // Write a config with mutations disabled.
    let mut cfg = crate::openhuman::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..crate::openhuman::config::Config::default()
    };
    cfg.update = crate::openhuman::config::UpdateConfig {
        rpc_mutations_enabled: false,
        ..crate::openhuman::config::UpdateConfig::default()
    };
    cfg.save().await.expect("save config");
    std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());

    let outcome = update_run().await;

    std::env::remove_var("OPENHUMAN_WORKSPACE");

    // Should contain an error indicating the mutation was blocked.
    let err = outcome.value.get("error");
    assert!(
        err.is_some(),
        "update_run with mutations disabled must return an error field"
    );
    let err_str = err.unwrap().as_str().unwrap_or("");
    assert!(
        err_str.contains("rpc_mutations_enabled=false") || err_str.contains("disabled"),
        "error message should mention the policy, got: {err_str}"
    );
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn sample_update_info(current: &str, latest: &str, update_available: bool) -> UpdateInfo {
    UpdateInfo {
        latest_version: latest.to_owned(),
        current_version: current.to_owned(),
        update_available,
        download_url: None,
        asset_name: None,
        release_notes: None,
        published_at: None,
    }
}
