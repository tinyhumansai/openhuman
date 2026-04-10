//! Core sidecar version checking and auto-update logic.
//!
//! After the Tauri shell starts the core sidecar, it queries `core.version` via
//! JSON-RPC. If the running core is older than the minimum expected version, the
//! shell downloads the latest release from GitHub, stages it, kills the old
//! process, and restarts with the new binary.

use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::core_process::CoreProcessHandle;

/// The minimum core version this Tauri build expects.
/// Bump this when the app depends on new core RPC methods.
pub const MINIMUM_CORE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// GitHub owner/repo for releases.
const GITHUB_OWNER: &str = "tinyhumansai";
const GITHUB_REPO: &str = "openhuman";

#[derive(Debug, Deserialize)]
struct RpcResponse {
    result: Option<serde_json::Value>,
    error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

/// Returned by `check_core_update` Tauri command.
#[derive(Debug, Clone, Serialize)]
pub struct CoreUpdateInfo {
    pub running_version: String,
    pub minimum_version: String,
    /// True if running < minimum (compatibility issue).
    pub outdated: bool,
    /// Latest version on GitHub Releases (if fetch succeeded).
    pub latest_version: Option<String>,
    /// True if running < latest (newer release available).
    pub update_available: bool,
}

/// Query the running core's version via JSON-RPC.
pub async fn query_core_version(rpc_url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| format!("http client error: {e}"))?;

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "core.version",
        "params": {}
    });

    let resp = client
        .post(rpc_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("failed to query core.version: {e}"))?;

    let rpc: RpcResponse = resp
        .json()
        .await
        .map_err(|e| format!("failed to parse core.version response: {e}"))?;

    if let Some(err) = rpc.error {
        return Err(format!("core.version RPC error: {err}"));
    }

    let version = rpc
        .result
        .and_then(|v| v.get("version").and_then(|v| v.as_str()).map(String::from))
        .ok_or_else(|| "core.version response missing 'version' field".to_string())?;

    Ok(version)
}

/// Compare two version strings. Returns true if `running` is older than `target`.
pub fn is_outdated(running: &str, target: &str) -> bool {
    let parse = |v: &str| -> Option<semver::Version> {
        semver::Version::parse(v.trim_start_matches('v')).ok()
    };
    match (parse(running), parse(target)) {
        (Some(r), Some(t)) => r < t,
        _ => {
            log::warn!("[core-update] could not parse versions running={running} target={target}");
            false
        }
    }
}

/// Full check: query running version, compare against minimum AND latest GitHub release.
pub async fn check_full(rpc_url: &str) -> Result<CoreUpdateInfo, String> {
    let running = query_core_version(rpc_url).await?;
    let minimum = MINIMUM_CORE_VERSION;
    let outdated = is_outdated(&running, minimum);

    // Best-effort fetch of latest release — don't fail the whole check if GitHub is unreachable.
    let (latest_version, update_available) = match fetch_latest_release().await {
        Ok(release) => {
            let latest = release.tag_name.trim_start_matches('v').to_string();
            let available = is_outdated(&running, &latest);
            (Some(latest), available)
        }
        Err(e) => {
            log::warn!("[core-update] could not fetch latest release: {e}");
            (None, false)
        }
    };

    Ok(CoreUpdateInfo {
        running_version: running,
        minimum_version: minimum.to_string(),
        outdated,
        latest_version,
        update_available,
    })
}

/// Build the platform triple for asset matching.
fn platform_triple() -> &'static str {
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

/// Find the right asset for this platform.
fn find_platform_asset(assets: &[GitHubAsset]) -> Option<&GitHubAsset> {
    let triple = platform_triple();
    let expected = format!("openhuman-core-{triple}");
    assets
        .iter()
        .find(|a| a.name == expected || a.name == format!("{expected}.exe"))
        .or_else(|| assets.iter().find(|a| a.name.starts_with(&expected)))
}

/// Fetch the latest release from GitHub.
async fn fetch_latest_release() -> Result<GitHubRelease, String> {
    let url = format!("https://api.github.com/repos/{GITHUB_OWNER}/{GITHUB_REPO}/releases/latest");

    let client = reqwest::Client::builder()
        .user_agent("openhuman-tauri-updater")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("http client error: {e}"))?;

    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("failed to fetch latest release: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("GitHub API error: {}", resp.status()));
    }

    resp.json()
        .await
        .map_err(|e| format!("failed to parse release: {e}"))
}

/// Download a binary from `url` and stage it at `dest`.
///
/// Uses a unique temp file (UUID-based) to avoid conflicts from concurrent downloads.
/// Sets executable permissions on Unix before the atomic rename.
async fn download_binary(url: &str, dest: &PathBuf) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .user_agent("openhuman-tauri-updater")
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| format!("http client error: {e}"))?;

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("download failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("download returned status {}", resp.status()));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("failed to read download: {e}"))?;

    log::info!(
        "[core-update] downloaded {} bytes to {}",
        bytes.len(),
        dest.display()
    );

    // Use a unique temp filename to avoid collisions from concurrent writes.
    let tmp_name = format!(
        ".openhuman-update-{}.tmp",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );
    let tmp = dest
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(tmp_name);

    {
        let mut file = std::fs::File::create(&tmp).map_err(|e| format!("create temp file: {e}"))?;
        file.write_all(&bytes)
            .map_err(|e| format!("write temp file: {e}"))?;
        file.flush().map_err(|e| format!("flush temp file: {e}"))?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("set permissions: {e}"))?;
    }

    std::fs::rename(&tmp, dest).map_err(|e| {
        // Best-effort cleanup of temp file on rename failure.
        let _ = std::fs::remove_file(&tmp);
        format!("rename staged binary: {e}")
    })?;

    Ok(())
}

/// The main auto-update flow, called after the core process starts.
///
/// When `force` is false (startup auto-check), only updates if the running core
/// is older than `MINIMUM_CORE_VERSION`. When `force` is true (manual trigger),
/// updates whenever GitHub has a newer version than what's currently running.
///
/// Emits Tauri events so the frontend can show progress.
pub async fn check_and_update_core(
    handle: CoreProcessHandle,
    app: Option<tauri::AppHandle>,
    force: bool,
) -> Result<(), String> {
    let rpc_url = handle.rpc_url();
    log::info!(
        "[core-update] checking core version at {} (minimum: {}, force: {})",
        rpc_url,
        MINIMUM_CORE_VERSION,
        force
    );

    // Step 1: Query running version.
    let running_version = match query_core_version(&rpc_url).await {
        Ok(v) => v,
        Err(e) => {
            log::warn!("[core-update] could not query core version: {e}");
            return Err(e);
        }
    };

    log::info!(
        "[core-update] running core version: {} (minimum: {})",
        running_version,
        MINIMUM_CORE_VERSION
    );

    let below_app_minimum = is_outdated(&running_version, MINIMUM_CORE_VERSION);
    if below_app_minimum {
        log::warn!(
            "[core-update] sidecar is OLDER than this app build (running {running_version}, need >= {min}). \
UI features (e.g. channel connect) may not match RPC until the core is updated.",
            min = MINIMUM_CORE_VERSION
        );
    }

    // Step 2: Fetch latest release from GitHub (needed to download a replacement binary).
    emit_event(&app, "core-update:status", "checking");

    let release = match fetch_latest_release().await {
        Ok(r) => r,
        Err(e) => {
            if force {
                log::warn!("[core-update] could not fetch latest release: {e}");
                return Err(e);
            }
            if below_app_minimum {
                log::error!(
                    "[core-update] cannot auto-update core (GitHub unreachable): {e}\n\
→ Stop any other `openhuman` / OpenHuman using RPC port {}.\n\
→ From repo root: `cargo build --manifest-path Cargo.toml --bin openhuman` then `cd app && yarn core:stage`, restart the app.\n\
→ Or fix network access to https://api.github.com (VPN/DNS/firewall).",
                    handle.port()
                );
                emit_event(&app, "core-update:status", "error");
                return Err(e);
            }
            log::warn!(
                "[core-update] could not fetch latest release (non-fatal; core meets minimum): {e}"
            );
            emit_event(&app, "core-update:status", "up_to_date");
            return Ok(());
        }
    };

    let latest_version = release.tag_name.trim_start_matches('v').to_string();
    log::info!("[core-update] latest release: {latest_version}");

    // Decide whether to proceed with the update.
    let needs_update = if force {
        // Manual trigger: update if GitHub has anything newer than what's running.
        is_outdated(&running_version, &latest_version)
    } else {
        // Auto-check: only update if running is below the minimum the app requires.
        is_outdated(&running_version, MINIMUM_CORE_VERSION)
    };

    if !needs_update {
        log::info!("[core-update] no update needed (running: {running_version}, latest: {latest_version}, force: {force})");
        emit_event(&app, "core-update:status", "up_to_date");
        return Ok(());
    }

    log::warn!(
        "[core-update] updating core {} → {} (force: {})",
        running_version,
        latest_version,
        force
    );

    let asset = find_platform_asset(&release.assets).ok_or_else(|| {
        format!(
            "no matching asset for platform '{}' in release {}",
            platform_triple(),
            latest_version
        )
    })?;

    log::info!(
        "[core-update] found asset: {} ({})",
        asset.name,
        asset.browser_download_url
    );

    emit_event(&app, "core-update:status", "downloading");

    // Step 3: Determine staging directory.
    let staging_dir = resolve_staging_dir();
    if let Some(ref dir) = staging_dir {
        if !dir.exists() {
            std::fs::create_dir_all(dir).map_err(|e| format!("create staging dir: {e}"))?;
        }
    }

    let dest = staging_dir
        .as_ref()
        .map(|d| d.join(&asset.name))
        .unwrap_or_else(|| PathBuf::from(&asset.name));

    // Step 4: Acquire restart lock, shutdown old process, download, stage, restart.
    // Hold the lock across download + staging + restart to prevent concurrent updates.
    {
        let _guard = handle.restart_lock().await;
        log::debug!("[core-update] acquired restart lock");

        // Shutdown old process first so the binary isn't in use during staging.
        handle.shutdown().await;

        // Wait for port to free.
        let mut waited = 0u64;
        while waited < 10_000 {
            if !port_open(handle.port()).await {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            waited += 50;
        }

        // Download and stage the new binary.
        download_binary(&asset.browser_download_url, &dest).await?;
        log::info!("[core-update] staged new binary at {}", dest.display());

        // Point the handle at the new binary so ensure_running launches it.
        handle.set_core_bin(dest).await;

        emit_event(&app, "core-update:status", "restarting");

        // Restart with the new binary.
        handle.ensure_running().await?;
    }

    log::info!(
        "[core-update] core updated from {} to {} and restarted",
        running_version,
        latest_version
    );

    emit_event(&app, "core-update:status", "updated");

    Ok(())
}

/// Resolve the directory where staged sidecar binaries are placed.
/// Mirrors the discovery logic in `core_process::default_core_bin()`.
fn resolve_staging_dir() -> Option<PathBuf> {
    // Dev: src-tauri/binaries/
    #[cfg(debug_assertions)]
    {
        let binaries_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries");
        if binaries_dir.exists() {
            return Some(binaries_dir);
        }
    }

    // Production: next to the executable, or Resources/ on macOS.
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;

    #[cfg(target_os = "macos")]
    {
        if let Some(resources) = exe_dir.parent().map(|p| p.join("Resources")) {
            if resources.exists() {
                return Some(resources);
            }
        }
    }

    Some(exe_dir.to_path_buf())
}

async fn port_open(port: u16) -> bool {
    matches!(
        tokio::time::timeout(
            std::time::Duration::from_millis(150),
            tokio::net::TcpStream::connect(("127.0.0.1", port)),
        )
        .await,
        Ok(Ok(_))
    )
}

fn emit_event(app: &Option<tauri::AppHandle>, event: &str, payload: &str) {
    if let Some(app) = app {
        use tauri::Emitter;
        if let Err(e) = app.emit(event, payload) {
            log::warn!("[core-update] failed to emit {event}: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outdated_detection() {
        assert!(is_outdated("0.49.17", "0.51.8"));
        assert!(is_outdated("0.50.0", "0.51.0"));
        assert!(!is_outdated("0.51.8", "0.51.8"));
        assert!(!is_outdated("0.52.0", "0.51.8"));
        assert!(!is_outdated("1.0.0", "0.51.8"));
    }
}
