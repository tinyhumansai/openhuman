use super::*;
use crate::openhuman::config::TEST_ENV_LOCK;
use tempfile::TempDir;

async fn write_update_policy(tmp: &TempDir, update: UpdateConfig) {
    let mut cfg = crate::openhuman::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..crate::openhuman::config::Config::default()
    };
    cfg.update = update;
    cfg.save().await.expect("save config");
}

// ── validate_download_url ─────────────────────────────────────

#[test]
fn validate_download_url_accepts_github_https_hosts() {
    for url in [
        "https://github.com/owner/repo/releases/download/v1/asset.tar.gz",
        "https://api.github.com/repos/owner/repo/releases/assets/1",
        "https://objects.githubusercontent.com/release-asset/123",
    ] {
        validate_download_url(url).unwrap_or_else(|e| panic!("`{url}` rejected: {e}"));
    }
}

#[test]
fn validate_download_url_rejects_non_github_hosts() {
    let err = validate_download_url("https://evil.example.com/asset.tar.gz").unwrap_err();
    assert!(err.contains("must be a GitHub domain"), "got: {err}");
}

#[test]
fn validate_download_url_rejects_non_https_schemes() {
    let err =
        validate_download_url("http://github.com/owner/repo/releases/download/v1/x").unwrap_err();
    assert!(err.contains("must use HTTPS"), "got: {err}");
}

#[test]
fn validate_download_url_rejects_malformed_url() {
    let err = validate_download_url("not a url").unwrap_err();
    assert!(err.contains("invalid download URL"), "got: {err}");
}

// ── validate_asset_name ───────────────────────────────────────

#[test]
fn validate_asset_name_accepts_well_formed_core_asset() {
    validate_asset_name("openhuman-core-aarch64-apple-darwin.tar.gz")
        .expect("canonical asset name should be accepted");
}

#[test]
fn validate_asset_name_rejects_empty_string() {
    let err = validate_asset_name("").unwrap_err();
    assert!(err.contains("must not be empty"));
}

#[test]
fn validate_asset_name_rejects_path_separators_and_traversal() {
    for bad in [
        "openhuman-core-../etc/passwd",
        "../openhuman-core-x86.tar.gz",
        "openhuman-core/x86.tar.gz",
        "openhuman-core\\x86.tar.gz",
    ] {
        let err = validate_asset_name(bad).unwrap_err();
        assert!(
            err.contains("path separators") || err.contains("'..'"),
            "input `{bad}` produced unexpected error: {err}"
        );
    }
}

#[test]
fn validate_asset_name_rejects_unprefixed_asset() {
    let err = validate_asset_name("malicious-binary.tar.gz").unwrap_err();
    assert!(
        err.contains("must start with 'openhuman-core-'"),
        "got: {err}"
    );
}

// ── update_apply rejection paths ──────────────────────────────

// `update_apply` reads the mutation-policy config from disk, whose
// path is resolved through the process-global `OPENHUMAN_WORKSPACE`
// env var. Tests that don't lock against the disabled-mutations
// case can race with it: the disabled test sets the env var, the
// sibling test (running on another thread) clears or shadows it
// between `WorkspaceEnvGuard::set` and the await inside
// `update_apply`, and the disabled test then loads a default
// policy (where `rpc_mutations_enabled = true`), proceeds past the
// gate, and fails its `contains("rpc_mutations_enabled=false")`
// assertion. Take `TEST_ENV_LOCK` in every test that calls
// `update_apply` so the three cases serialise on the same mutex.
#[tokio::test]
async fn update_apply_rejects_non_github_url_before_network_call() {
    let _guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let outcome = update_apply(
        "https://evil.example.com/asset".to_string(),
        "openhuman-core-x86_64.tar.gz".to_string(),
        None,
    )
    .await;
    assert!(outcome.value.get("error").is_some());
    assert!(outcome
        .logs
        .iter()
        .any(|l| l.contains("update_apply rejected")));
}

#[tokio::test]
async fn update_apply_rejects_unsafe_asset_name() {
    let _guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let outcome = update_apply(
        "https://github.com/owner/repo/releases/download/v1/x".to_string(),
        "../etc/passwd".to_string(),
        None,
    )
    .await;
    assert!(outcome.value.get("error").is_some());
    assert!(outcome
        .logs
        .iter()
        .any(|l| l.contains("update_apply rejected")));
}

struct WorkspaceEnvGuard;
impl WorkspaceEnvGuard {
    fn set(path: &std::path::Path) -> Self {
        std::env::set_var("OPENHUMAN_WORKSPACE", path);
        Self
    }
}
impl Drop for WorkspaceEnvGuard {
    fn drop(&mut self) {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[tokio::test]
async fn update_apply_rejects_when_rpc_mutations_disabled() {
    let _guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let _workspace_guard = WorkspaceEnvGuard::set(tmp.path());
    write_update_policy(
        &tmp,
        UpdateConfig {
            rpc_mutations_enabled: false,
            ..UpdateConfig::default()
        },
    )
    .await;

    let outcome = update_apply(
        "https://github.com/owner/repo/releases/download/v1/x".to_string(),
        "openhuman-core-x86_64.tar.gz".to_string(),
        None,
    )
    .await;

    assert!(outcome.value.get("error").is_some());
    assert!(outcome.value["error"]
        .as_str()
        .is_some_and(|value| value.contains("rpc_mutations_enabled=false")));
}

#[tokio::test]
async fn supervisor_restart_strategy_stages_without_restart_request() {
    let info = UpdateInfo {
        latest_version: "9.9.9".into(),
        current_version: "1.0.0".into(),
        update_available: true,
        download_url: Some(
            "https://github.com/owner/repo/releases/download/v9/openhuman-core".into(),
        ),
        asset_name: Some("openhuman-core-x86_64-unknown-linux-gnu".into()),
        release_notes: None,
        published_at: None,
    };
    let applied = UpdateApplyResult {
        installed_version: "9.9.9".into(),
        staged_path: "/tmp/openhuman-core".into(),
        restart_required: true,
        restart_strategy: UpdateRestartStrategy::SelfReplace,
    };

    let result =
        build_run_result_from_staged_update(info, applied, UpdateRestartStrategy::Supervisor).await;

    assert!(result.applied);
    assert!(!result.restart_requested);
    assert_eq!(result.restart_strategy, UpdateRestartStrategy::Supervisor);
    assert!(result.message.contains("supervisor restart required"));
}

// NOTE: `update_check` and the success path of `update_apply`
// hit GitHub's REST API and stage real binaries on disk — they
// are deferred to the integration test suite (tests/) where a
// real network fixture or recorded cassette is available.
