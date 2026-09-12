//! JSON-RPC / CLI controller surface for the update domain.

use std::path::PathBuf;

use serde_json::Value;

use crate::openhuman::config::{self, UpdateConfig, UpdateRestartStrategy};
use crate::openhuman::platform::update;
use crate::openhuman::platform::update::types::{
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
            let restart_requested = match crate::openhuman::platform::service::rpc::service_restart(
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
#[path = "ops_tests.rs"]
mod ops_tests;

#[cfg(test)]
#[path = "ops_tests_2_tests.rs"]
mod tests;
