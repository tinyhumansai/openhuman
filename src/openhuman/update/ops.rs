//! JSON-RPC / CLI controller surface for the update domain.

use std::path::PathBuf;

use serde_json::Value;

use crate::openhuman::config::{self, UpdateConfig, UpdateRestartStrategy};
use crate::openhuman::update;
use crate::openhuman::update::types::{
    UpdateApplyResult, UpdateInfo, UpdateRunResult, VersionInfo,
};
use crate::rpc::RpcOutcome;

async fn load_update_policy() -> Result<UpdateConfig, String> {
    config::rpc::load_config_with_timeout()
        .await
        .map(|cfg| cfg.update)
        .map_err(|err| format!("failed to load config for update policy: {err}"))
}

async fn enforce_update_mutation_policy(method: &str) -> Result<UpdateConfig, String> {
    let policy = load_update_policy().await.map_err(|err| {
        let message = format!(
            "{method} blocked: {err}; failing closed because update policy could not be loaded"
        );
        log::error!("[update:rpc] {}", message);
        message
    })?;
    if policy.rpc_mutations_enabled {
        return Ok(policy);
    }

    let message = format!(
        "{method} is disabled by config.update.rpc_mutations_enabled=false; \
         use update.check for discovery and restart under a supervisor after staging manually"
    );
    log::warn!("[update:rpc] {}", message);
    Err(message)
}

fn already_current_result(
    info: &UpdateInfo,
    restart_strategy: UpdateRestartStrategy,
) -> UpdateRunResult {
    UpdateRunResult {
        current_version: info.current_version.clone(),
        latest_version: info.latest_version.clone(),
        update_available: false,
        applied: false,
        staged_path: None,
        restart_requested: false,
        restart_strategy,
        message: format!("already on latest ({})", info.current_version),
    }
}

fn missing_asset_result(
    info: UpdateInfo,
    restart_strategy: UpdateRestartStrategy,
) -> UpdateRunResult {
    UpdateRunResult {
        current_version: info.current_version,
        latest_version: info.latest_version,
        update_available: true,
        applied: false,
        staged_path: None,
        restart_requested: false,
        restart_strategy,
        message: format!(
            "latest release has no asset for target {}",
            update::platform_triple()
        ),
    }
}

fn apply_failure_result(
    info: UpdateInfo,
    restart_strategy: UpdateRestartStrategy,
    error: &str,
) -> UpdateRunResult {
    UpdateRunResult {
        current_version: info.current_version,
        latest_version: info.latest_version,
        update_available: true,
        applied: false,
        staged_path: None,
        restart_requested: false,
        restart_strategy,
        message: format!("download/stage failed: {error}"),
    }
}

async fn build_run_result_from_staged_update(
    info: UpdateInfo,
    mut applied: UpdateApplyResult,
    restart_strategy: UpdateRestartStrategy,
) -> UpdateRunResult {
    applied.restart_strategy = restart_strategy;

    match restart_strategy {
        UpdateRestartStrategy::SelfReplace => {
            let restart_requested = match crate::openhuman::service::rpc::service_restart(
                Some("update.run".to_string()),
                Some(format!("update to {}", info.latest_version)),
            )
            .await
            {
                Ok(_) => true,
                Err(e) => {
                    log::warn!(
                        "[update:rpc] update_run staged update but restart publish failed: {}",
                        e
                    );
                    false
                }
            };

            UpdateRunResult {
                current_version: info.current_version,
                latest_version: info.latest_version,
                update_available: true,
                applied: true,
                staged_path: Some(applied.staged_path.clone()),
                restart_requested,
                restart_strategy,
                message: if restart_requested {
                    format!(
                        "staged {} — restart requested",
                        applied.staged_path.as_str()
                    )
                } else {
                    format!(
                        "staged {} — restart publish failed; caller must restart manually",
                        applied.staged_path.as_str()
                    )
                },
            }
        }
        UpdateRestartStrategy::Supervisor => UpdateRunResult {
            current_version: info.current_version,
            latest_version: info.latest_version.clone(),
            update_available: true,
            applied: true,
            staged_path: Some(applied.staged_path.clone()),
            restart_requested: false,
            restart_strategy,
            message: format!(
                "staged {} — supervisor restart required before {} takes effect",
                applied.staged_path.as_str(),
                info.latest_version
            ),
        },
    }
}

/// Report the running core binary's version + target triple.
///
/// Cheap, no-network — the frontend uses this to decide whether to
/// invoke the heavier `update.check` or `update.run` RPCs.
pub async fn update_version() -> RpcOutcome<Value> {
    let info = VersionInfo {
        version: update::current_version().to_string(),
        target_triple: update::platform_triple().to_string(),
        asset_prefix: format!("openhuman-core-{}", update::platform_triple()),
    };
    log::debug!(
        "[update:rpc] update_version → {} ({})",
        info.version,
        info.target_triple
    );
    let value = serde_json::to_value(&info)
        .unwrap_or_else(|e| serde_json::json!({ "error": format!("serialization failed: {e}") }));
    RpcOutcome::single_log(value, "update_version completed")
}

/// Orchestrated update flow: check → apply (if newer) → restart.
///
/// Returns an `UpdateRunResult` describing what happened. When an
/// update was applied the function publishes a restart request before
/// returning, so the caller will see `restart_requested: true` and the
/// core process will exit shortly afterwards.
pub async fn update_run() -> RpcOutcome<Value> {
    log::info!("[update:rpc] update_run invoked");
    let policy = match enforce_update_mutation_policy("openhuman.update_run").await {
        Ok(policy) => policy,
        Err(error) => {
            return RpcOutcome::single_log(
                serde_json::json!({
                    "error": error,
                    "applied": false,
                    "restart_requested": false,
                }),
                "update_run rejected by policy",
            );
        }
    };
    let restart_strategy = policy.restart_strategy;

    let info = match update::check_available().await {
        Ok(i) => i,
        Err(e) => {
            log::error!("[update:rpc] update_run check failed: {e}");
            return RpcOutcome::single_log(
                serde_json::json!({
                    "error": e,
                    "applied": false,
                    "restart_requested": false,
                }),
                format!("update_run: check failed: {e}"),
            );
        }
    };

    if !info.update_available {
        let result = already_current_result(&info, restart_strategy);
        log::info!(
            "[update:rpc] update_run: already up to date ({})",
            result.current_version
        );
        return RpcOutcome::single_log(
            serde_json::to_value(&result).unwrap_or(Value::Null),
            "update_run: already up to date",
        );
    }

    let (Some(download_url), Some(asset_name)) =
        (info.download_url.clone(), info.asset_name.clone())
    else {
        log::warn!(
            "[update:rpc] update_run: latest release has no asset for this platform (target={})",
            update::platform_triple()
        );
        let result = missing_asset_result(info, restart_strategy);
        return RpcOutcome::single_log(
            serde_json::to_value(&result).unwrap_or(Value::Null),
            "update_run: missing platform asset",
        );
    };

    // Defensive re-validation — the URL/asset came from GitHub but we
    // still gate them through the same checks `update.apply` uses, so
    // this orchestrator can't accidentally bypass the safety net.
    if let Err(e) = validate_download_url(&download_url) {
        log::error!("[update:rpc] update_run rejected download URL: {e}");
        return RpcOutcome::single_log(
            serde_json::json!({ "error": e, "applied": false, "restart_requested": false }),
            format!("update_run rejected: {e}"),
        );
    }
    if let Err(e) = validate_asset_name(&asset_name) {
        log::error!("[update:rpc] update_run rejected asset name: {e}");
        return RpcOutcome::single_log(
            serde_json::json!({ "error": e, "applied": false, "restart_requested": false }),
            format!("update_run rejected: {e}"),
        );
    }

    let applied = match update::download_and_stage(&download_url, &asset_name, None).await {
        Ok(r) => r,
        Err(e) => {
            log::error!("[update:rpc] update_run apply failed: {e}");
            let result = apply_failure_result(info, restart_strategy, &e);
            return RpcOutcome::single_log(
                serde_json::to_value(&result).unwrap_or(Value::Null),
                format!("update_run: apply failed: {e}"),
            );
        }
    };

    let result = build_run_result_from_staged_update(info, applied, restart_strategy).await;
    log::info!(
        "[update:rpc] update_run completed applied=true restart_requested={} restart_strategy={:?}",
        result.restart_requested,
        result.restart_strategy
    );
    RpcOutcome::single_log(
        serde_json::to_value(&result).unwrap_or(Value::Null),
        "update_run completed",
    )
}

/// Check GitHub Releases for a newer version of the core binary.
pub async fn update_check() -> RpcOutcome<Value> {
    log::info!("[update:rpc] update_check invoked");
    match update::check_available().await {
        Ok(info) => {
            let value = serde_json::to_value(&info).unwrap_or_else(
                |e| serde_json::json!({ "error": format!("serialization failed: {e}") }),
            );
            RpcOutcome::single_log(value, "update_check completed")
        }
        Err(e) => {
            log::error!("[update:rpc] update_check failed: {e}");
            RpcOutcome::single_log(
                serde_json::json!({ "error": e }),
                format!("update_check failed: {e}"),
            )
        }
    }
}

/// Validate that a download URL points to a GitHub release asset.
fn validate_download_url(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("invalid download URL: {e}"))?;

    let host = parsed.host_str().unwrap_or("");
    if host != "github.com" && host != "api.github.com" && !host.ends_with(".githubusercontent.com")
    {
        return Err(format!(
            "download URL must be a GitHub domain, got '{host}'"
        ));
    }

    if parsed.scheme() != "https" {
        return Err("download URL must use HTTPS".to_string());
    }

    Ok(())
}

/// Validate asset_name is a safe filename (no path separators or traversal).
fn validate_asset_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("asset_name must not be empty".to_string());
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(format!(
            "asset_name must not contain path separators or '..', got '{name}'"
        ));
    }
    if !name.starts_with("openhuman-core-") {
        return Err(format!(
            "asset_name must start with 'openhuman-core-', got '{name}'"
        ));
    }
    Ok(())
}

/// Download and stage the updated binary to a given path.
///
/// Params:
///   - `download_url` (string, required): must be a GitHub release asset URL (HTTPS).
///   - `asset_name` (string, required): must be a safe filename starting with `openhuman-core-`.
///   - `staging_dir` (string, optional): ignored — always uses the default staging directory
///     for security (next to the running executable or Resources/).
pub async fn update_apply(
    download_url: String,
    asset_name: String,
    _staging_dir: Option<String>,
) -> RpcOutcome<Value> {
    log::info!(
        "[update:rpc] update_apply invoked — url={} asset={}",
        download_url,
        asset_name,
    );
    let policy = match enforce_update_mutation_policy("openhuman.update_apply").await {
        Ok(policy) => policy,
        Err(error) => {
            return RpcOutcome::single_log(
                serde_json::json!({ "error": error }),
                "update_apply rejected by policy",
            );
        }
    };

    // Validate inputs at the RPC boundary.
    if let Err(e) = validate_download_url(&download_url) {
        log::error!("[update:rpc] rejected download URL: {e}");
        return RpcOutcome::single_log(
            serde_json::json!({ "error": e }),
            format!("update_apply rejected: {e}"),
        );
    }
    if let Err(e) = validate_asset_name(&asset_name) {
        log::error!("[update:rpc] rejected asset name: {e}");
        return RpcOutcome::single_log(
            serde_json::json!({ "error": e }),
            format!("update_apply rejected: {e}"),
        );
    }

    // Ignore caller-provided staging_dir — always use the safe default.
    let dir: Option<PathBuf> = None;
    match update::download_and_stage(&download_url, &asset_name, dir).await {
        Ok(mut result) => {
            result.restart_strategy = policy.restart_strategy;
            let value = serde_json::to_value(&result).unwrap_or_else(
                |e| serde_json::json!({ "error": format!("serialization failed: {e}") }),
            );
            RpcOutcome::single_log(value, "update_apply completed")
        }
        Err(e) => {
            log::error!("[update:rpc] update_apply failed: {e}");
            RpcOutcome::single_log(
                serde_json::json!({ "error": e }),
                format!("update_apply failed: {e}"),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::TEST_ENV_LOCK;
    use tempfile::TempDir;

    async fn write_update_policy(tmp: &TempDir, update: UpdateConfig) {
        let mut cfg = crate::openhuman::config::Config {
            workspace_dir: tmp.path().join("workspace"),
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
        let err = validate_download_url("http://github.com/owner/repo/releases/download/v1/x")
            .unwrap_err();
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
            build_run_result_from_staged_update(info, applied, UpdateRestartStrategy::Supervisor)
                .await;

        assert!(result.applied);
        assert!(!result.restart_requested);
        assert_eq!(result.restart_strategy, UpdateRestartStrategy::Supervisor);
        assert!(result.message.contains("supervisor restart required"));
    }

    // NOTE: `update_check` and the success path of `update_apply`
    // hit GitHub's REST API and stage real binaries on disk — they
    // are deferred to the integration test suite (tests/) where a
    // real network fixture or recorded cassette is available.
}
