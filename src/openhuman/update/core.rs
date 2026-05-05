//! Core self-update logic: check GitHub Releases for a newer `openhuman-core` binary
//! and download + stage it for the Tauri shell to swap in.

use std::io::Write;
use std::path::PathBuf;

use crate::openhuman::update::types::{GitHubAsset, GitHubRelease, UpdateApplyResult, UpdateInfo};

/// GitHub owner/repo for the core binary releases.
const GITHUB_OWNER: &str = "tinyhumansai";
const GITHUB_REPO: &str = "openhuman";

/// Current binary version (set at compile time from Cargo.toml).
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Build the target triple string used in release asset names.
/// E.g. `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`.
pub fn platform_triple() -> &'static str {
    #[cfg(all(target_arch = "x86_64", target_os = "macos"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(all(target_arch = "aarch64", target_os = "macos"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    {
        "x86_64-unknown-linux-gnu"
    }
    #[cfg(all(target_arch = "aarch64", target_os = "linux"))]
    {
        "aarch64-unknown-linux-gnu"
    }
    #[cfg(all(target_arch = "x86_64", target_os = "windows"))]
    {
        "x86_64-pc-windows-msvc"
    }
    #[cfg(all(target_arch = "aarch64", target_os = "windows"))]
    {
        "aarch64-pc-windows-msvc"
    }
}

/// Find the right asset for this platform from a list of release assets.
///
/// Convention: assets are named `openhuman-core-{triple}` (or `.exe` on Windows).
fn find_platform_asset(assets: &[GitHubAsset]) -> Option<&GitHubAsset> {
    let triple = platform_triple();
    let expected_name = format!("openhuman-core-{triple}");

    log::debug!(
        "[update] looking for asset matching '{}' among {} assets",
        expected_name,
        assets.len()
    );

    // Try exact match first, then prefix match.
    assets
        .iter()
        .find(|a| a.name == expected_name || a.name == format!("{expected_name}.exe"))
        .or_else(|| assets.iter().find(|a| a.name.starts_with(&expected_name)))
}

/// Compare two semver-ish version strings.
/// Returns true if `latest` is newer than `current`.
fn is_newer(latest: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v')
            .split('.')
            .filter_map(|s| s.parse::<u64>().ok())
            .collect()
    };
    let l = parse(latest);
    let c = parse(current);
    l > c
}

/// Check GitHub Releases for a newer version of openhuman-core.
pub async fn check_available() -> Result<UpdateInfo, String> {
    let current = current_version();
    log::info!(
        "[update] checking for updates — current version: {}",
        current
    );

    let url = format!("https://api.github.com/repos/{GITHUB_OWNER}/{GITHUB_REPO}/releases/latest");

    let client = reqwest::Client::builder()
        .user_agent("openhuman-core-updater")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;

    let response = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("failed to fetch latest release: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_else(|_| "(no body)".into());
        log::warn!(
            "[update] GitHub API returned {}: {}",
            status,
            &body[..body.len().min(200)]
        );
        return Err(format!("GitHub API error: {status}"));
    }

    let release: GitHubRelease = response
        .json()
        .await
        .map_err(|e| format!("failed to parse release JSON: {e}"))?;

    let latest_version = release.tag_name.trim_start_matches('v').to_string();
    let update_available = is_newer(&latest_version, current);
    let platform_asset = find_platform_asset(&release.assets);

    let info = UpdateInfo {
        latest_version,
        current_version: current.to_string(),
        update_available,
        download_url: platform_asset.map(|a| a.browser_download_url.clone()),
        asset_name: platform_asset.map(|a| a.name.clone()),
        release_notes: release.body,
        published_at: release.published_at,
    };

    log::info!(
        "[update] check complete — latest={} current={} update_available={} asset={}",
        info.latest_version,
        info.current_version,
        info.update_available,
        info.asset_name.as_deref().unwrap_or("(none)")
    );

    Ok(info)
}

/// Download and stage the updated binary.
///
/// The binary is downloaded to a temp file, then moved to the staging path.
/// The caller (Tauri shell) is responsible for killing the old process and
/// restarting with the new binary.
///
/// `staging_dir` — directory where the new binary should be placed (e.g.
/// the `binaries/` dir next to the Tauri app, or the Resources dir).
/// If `None`, uses the directory of the currently running executable.
///
/// `target_version` — the version of the release being staged, used in the
/// returned `UpdateApplyResult`. If `None`, falls back to `current_version()`.
pub async fn download_and_stage(
    download_url: &str,
    asset_name: &str,
    staging_dir: Option<PathBuf>,
) -> Result<UpdateApplyResult, String> {
    download_and_stage_with_version(download_url, asset_name, staging_dir, None).await
}

pub async fn download_and_stage_with_version(
    download_url: &str,
    asset_name: &str,
    staging_dir: Option<PathBuf>,
    target_version: Option<&str>,
) -> Result<UpdateApplyResult, String> {
    log::info!(
        "[update] downloading update from {} (asset: {})",
        download_url,
        asset_name
    );

    let client = reqwest::Client::builder()
        .user_agent("openhuman-core-updater")
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;

    let response = client
        .get(download_url)
        .send()
        .await
        .map_err(|e| format!("failed to download update: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("download failed with status {}", response.status()));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("failed to read update body: {e}"))?;

    log::info!("[update] downloaded {} bytes", bytes.len());

    // Determine staging path.
    let dir = if let Some(d) = staging_dir {
        d
    } else {
        std::env::current_exe()
            .map_err(|e| format!("cannot resolve current exe: {e}"))?
            .parent()
            .ok_or_else(|| "cannot resolve exe parent dir".to_string())?
            .to_path_buf()
    };

    if !dir.exists() {
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("failed to create staging dir {}: {e}", dir.display()))?;
    }

    let staged_path = dir.join(asset_name);

    // Write to a temp file first, then rename for atomicity.
    let tmp_path = dir.join(format!(".{asset_name}.tmp"));
    {
        let mut file = std::fs::File::create(&tmp_path)
            .map_err(|e| format!("failed to create temp file: {e}"))?;
        file.write_all(&bytes)
            .map_err(|e| format!("failed to write update binary: {e}"))?;
        file.flush()
            .map_err(|e| format!("failed to flush update binary: {e}"))?;
    }

    // Set executable permission on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("failed to set executable permission: {e}"))?;
    }

    // Atomic rename (same filesystem).
    std::fs::rename(&tmp_path, &staged_path)
        .map_err(|e| format!("failed to move update to {}: {e}", staged_path.display()))?;

    let installed_version = target_version
        .unwrap_or_else(|| current_version())
        .to_string();

    log::info!("[update] staged update binary at {}", staged_path.display());

    Ok(UpdateApplyResult {
        installed_version,
        staged_path: staged_path.to_string_lossy().to_string(),
        restart_required: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_newer_detects_update() {
        assert!(is_newer("0.50.0", "0.49.17"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(is_newer("v0.50.0", "0.49.17"));
        assert!(!is_newer("0.49.17", "0.49.17"));
        assert!(!is_newer("0.49.16", "0.49.17"));
        assert!(!is_newer("0.49.17", "0.50.0"));
    }

    #[test]
    fn current_version_is_not_empty() {
        assert!(!current_version().is_empty());
    }
}
