use sha2::{Digest, Sha256};

use crate::openhuman::config::{Config, UpdateMode};
use crate::rpc::RpcOutcome;

use super::resolver::{compare_versions, fetch_latest_release};
use super::store::{
    has_staged_update, managed_binary_path, staged_binary_path, write_staged_binary,
};
use super::types::{UpdateApplyStatus, UpdateAsset, UpdateCheckStatus};

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn should_prompt(mode: UpdateMode, latest: &Option<UpdateAsset>, dismissed: Option<&str>) -> bool {
    if !matches!(mode, UpdateMode::Prompt) {
        return false;
    }
    let Some(latest) = latest else {
        return false;
    };
    dismissed != Some(latest.version.as_str())
}

fn normalize_digest(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

async fn download_asset(url: &str) -> Result<Vec<u8>, String> {
    log::debug!("[update] downloading asset from {url}");
    let client = reqwest::Client::builder()
        .build()
        .map_err(|e| format!("failed to build http client: {e}"))?;

    let response = client
        .get(url)
        .header("User-Agent", "openhuman-core-updater")
        .send()
        .await
        .map_err(|e| format!("failed to download update asset: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "update asset download failed with status {}",
            response.status().as_u16()
        ));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("failed to read update asset bytes: {e}"))?;
    log::debug!("[update] downloaded {} bytes", bytes.len());
    Ok(bytes.to_vec())
}

fn verify_digest(bytes: &[u8], expected_sha256: Option<&str>) -> Result<(), String> {
    let Some(expected) = expected_sha256 else {
        log::debug!("[update] no digest provided, skipping verification");
        return Ok(());
    };
    log::debug!("[update] verifying SHA256 digest");

    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = format!("{:x}", hasher.finalize());

    if actual != normalize_digest(expected) {
        return Err(format!(
            "update asset checksum mismatch: expected {}, got {}",
            normalize_digest(expected),
            actual
        ));
    }

    Ok(())
}

pub fn apply_staged_update_preflight() -> Result<bool, String> {
    let target = managed_binary_path()?;
    log::debug!("[update] preflight: checking for staged update at {}", target.display());
    match super::store::apply_staged_update_for_path(&target) {
        Ok(true) => {
            log::info!("[update] preflight: staged update applied successfully");
            Ok(true)
        }
        Ok(false) => {
            log::debug!("[update] preflight: no staged update found");
            Ok(false)
        }
        Err(error) => {
            #[cfg(windows)]
            {
                log::warn!(
                    "[update] staged update present but could not be activated yet (will retry): {error}"
                );
                return Ok(false);
            }
            #[cfg(not(windows))]
            {
                Err(error)
            }
        }
    }
}

async fn check_for_update(config: &mut Config) -> Result<Option<UpdateAsset>, String> {
    let current_version = env!("CARGO_PKG_VERSION");
    log::debug!("[update] checking for update (current={current_version})");
    let resolved = fetch_latest_release(config.update.last_etag.as_deref()).await?;

    config.update.last_check_at = Some(now_rfc3339());
    config.update.last_error = None;

    if resolved.not_modified {
        log::debug!("[update] release not modified (ETag match)");
        config.update.last_result = Some("not_modified".to_string());
        return Ok(None);
    }

    if let Some(etag) = resolved.etag {
        config.update.last_etag = Some(etag);
    }

    let latest = resolved.latest;
    if let Some(asset) = latest.as_ref() {
        config.update.last_seen_version = Some(asset.version.clone());
        let ordering = compare_versions(&asset.version, current_version)?;
        log::debug!(
            "[update] version compare: latest={} current={current_version} ordering={ordering:?}",
            asset.version
        );
        if ordering.is_gt() {
            config.update.last_result = Some("update_available".to_string());
            return Ok(latest);
        }
    }

    log::debug!("[update] already up to date");
    config.update.last_result = Some("up_to_date".to_string());
    Ok(None)
}

pub async fn update_status() -> Result<RpcOutcome<UpdateCheckStatus>, String> {
    let config = Config::load_or_init().await.map_err(|e| e.to_string())?;
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let target_bin = managed_binary_path()?;
    let pending_restart = has_staged_update(&target_bin);
    let staged_path = if pending_restart {
        Some(staged_binary_path(&target_bin).display().to_string())
    } else {
        None
    };

    let latest = config.update.last_seen_version.clone().and_then(|version| {
        if compare_versions(&version, &current_version).ok()?.is_gt() {
            Some(UpdateAsset {
                version,
                tag: String::new(),
                name: String::new(),
                download_url: String::new(),
                digest_sha256: None,
                release_url: String::new(),
            })
        } else {
            None
        }
    });

    let status = UpdateCheckStatus {
        current_version,
        mode: config.update.mode,
        check_interval_hours: config.update.check_interval_hours,
        last_check_at: config.update.last_check_at.clone(),
        last_seen_version: config.update.last_seen_version.clone(),
        last_result: config.update.last_result.clone(),
        last_error: config.update.last_error.clone(),
        update_available: latest.is_some(),
        should_prompt: should_prompt(
            config.update.mode,
            &latest,
            config.update.last_dismissed_version.as_deref(),
        ),
        latest,
        pending_restart,
        staged_path,
    };

    Ok(RpcOutcome::single_log(status, "update status resolved"))
}

pub async fn update_set_policy(
    mode: UpdateMode,
    check_interval_hours: Option<u64>,
) -> Result<RpcOutcome<UpdateCheckStatus>, String> {
    let mut config = Config::load_or_init().await.map_err(|e| e.to_string())?;
    config.update.mode = mode;
    if let Some(hours) = check_interval_hours {
        config.update.check_interval_hours = hours.max(1);
    }
    config.save().await.map_err(|e| e.to_string())?;
    update_status().await
}

pub async fn update_dismiss(version: String) -> Result<RpcOutcome<UpdateCheckStatus>, String> {
    log::debug!("[update] dismissing version {version}");
    let mut config = Config::load_or_init().await.map_err(|e| e.to_string())?;
    config.update.last_dismissed_version = Some(version);
    config.save().await.map_err(|e| e.to_string())?;
    update_status().await
}

pub async fn update_check() -> Result<RpcOutcome<UpdateCheckStatus>, String> {
    let mut config = Config::load_or_init().await.map_err(|e| e.to_string())?;
    match check_for_update(&mut config).await {
        Ok(_) => {}
        Err(error) => {
            config.update.last_check_at = Some(now_rfc3339());
            config.update.last_result = Some("error".to_string());
            config.update.last_error = Some(error.clone());
            config.save().await.map_err(|e| e.to_string())?;
            return Err(error);
        }
    }

    config.save().await.map_err(|e| e.to_string())?;
    update_status().await
}

async fn download_and_stage(asset: &UpdateAsset) -> Result<std::path::PathBuf, String> {
    let bytes = download_asset(&asset.download_url).await?;
    verify_digest(&bytes, asset.digest_sha256.as_deref())?;
    let target_bin = managed_binary_path()?;
    write_staged_binary(&target_bin, &bytes)
}

pub async fn update_apply() -> Result<RpcOutcome<UpdateApplyStatus>, String> {
    let mut config = Config::load_or_init().await.map_err(|e| e.to_string())?;
    let latest = check_for_update(&mut config).await?;
    let asset = latest.ok_or_else(|| "no newer update is available".to_string())?;

    let staged_path = download_and_stage(&asset).await?;

    config.update.last_result = Some("staged".to_string());
    config.update.last_error = None;
    config.save().await.map_err(|e| e.to_string())?;

    let out = UpdateApplyStatus {
        staged_path: staged_path.display().to_string(),
        pending_restart: true,
        version: asset.version,
        release_url: asset.release_url,
    };

    Ok(RpcOutcome::single_log(
        out,
        "update downloaded, verified, and staged",
    ))
}

pub async fn maybe_background_check() {
    log::debug!("[update] evaluating background check");
    let mut config = match Config::load_or_init().await {
        Ok(config) => config,
        Err(error) => {
            log::warn!("[update] failed to load config for background check: {error}");
            return;
        }
    };

    if matches!(config.update.mode, UpdateMode::Manual) {
        log::debug!("[update] background check skipped (mode=manual)");
        return;
    }

    let due = config
        .update
        .last_check_at
        .as_deref()
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
        .map(|v| chrono::Utc::now() - v.with_timezone(&chrono::Utc))
        .map(|elapsed| elapsed.num_hours() >= config.update.check_interval_hours as i64)
        .unwrap_or(true);

    if !due {
        log::debug!("[update] background check not yet due");
        return;
    }
    log::debug!("[update] background check is due, running now");

    match check_for_update(&mut config).await {
        Ok(Some(asset)) if matches!(config.update.mode, UpdateMode::Auto) => {
            log::debug!(
                "[update] auto mode: downloading and staging version {}",
                asset.version
            );
            match download_and_stage(&asset).await {
                Ok(staged_path) => {
                    config.update.last_result = Some("staged".to_string());
                    config.update.last_error = None;
                    log::info!(
                        "[update] auto mode: staged version {} at {}",
                        asset.version,
                        staged_path.display()
                    );
                }
                Err(error) => {
                    config.update.last_error = Some(error.clone());
                    log::warn!("[update] auto mode: stage failed: {error}");
                }
            }
            if let Err(error) = config.save().await {
                log::warn!("[update] failed to persist background check result: {error}");
            }
        }
        Ok(_) => {
            if let Err(error) = config.save().await {
                log::warn!("[update] failed to persist background check result: {error}");
            }
        }
        Err(error) => {
            config.update.last_result = Some("error".to_string());
            config.update.last_error = Some(error.clone());
            config.update.last_check_at = Some(now_rfc3339());
            let _ = config.save().await;
            log::warn!("[update] background check failed: {error}");
        }
    }
}
