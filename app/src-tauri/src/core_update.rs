//! Core sidecar version checking and auto-update logic.
//!
//! After the Tauri shell starts the core sidecar, it queries `core.version` via
//! JSON-RPC. If the running core is older than the minimum expected version, the
//! shell downloads the latest release from GitHub, stages it, kills the old
//! process, and restarts with the new binary.

use std::io::Write;
use std::path::{Path, PathBuf};

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
///
/// Current release format (since 0.52.x): `openhuman-core-<version>-<triple>.tar.gz`
/// on Unix, `.zip` on Windows. The archive contains a single `openhuman-core`
/// (or `openhuman-core.exe`) file with no wrapping directory.
///
/// Legacy format (kept as fallback for older releases): a raw binary named
/// `openhuman-core-<triple>` (or `.exe`).
fn find_platform_asset(assets: &[GitHubAsset]) -> Option<&GitHubAsset> {
    let triple = platform_triple();
    let archive_ext = if cfg!(windows) { ".zip" } else { ".tar.gz" };

    // New versioned-archive format: `openhuman-core-0.52.26-aarch64-apple-darwin.tar.gz`
    let archive_match = assets.iter().find(|a| {
        a.name.starts_with("openhuman-core-")
            && a.name.contains(triple)
            && a.name.ends_with(archive_ext)
            // Defensive: avoid matching detached signatures or checksums that
            // happen to share the prefix (e.g. `…tar.gz.sha256`, `…tar.gz.sig`).
            && !a.name.ends_with(".sha256")
            && !a.name.ends_with(".sig")
    });
    if archive_match.is_some() {
        return archive_match;
    }

    // Legacy raw-binary format.
    let legacy = format!("openhuman-core-{triple}");
    let legacy_exe = format!("{legacy}.exe");
    assets
        .iter()
        .find(|a| a.name == legacy || a.name == legacy_exe)
}

/// Filename the staged core binary must be saved as for `core_process::default_core_bin`
/// to discover it on subsequent runs.
fn staged_binary_name() -> String {
    let triple = platform_triple();
    if cfg!(windows) {
        format!("openhuman-core-{triple}.exe")
    } else {
        format!("openhuman-core-{triple}")
    }
}

/// True if the asset name looks like an archive we need to extract (vs. a raw binary).
fn is_archive_asset(name: &str) -> bool {
    name.ends_with(".tar.gz") || name.ends_with(".tgz") || name.ends_with(".zip")
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

/// Build a unique sibling temp path next to `dest` to stage writes before an atomic rename.
fn unique_tmp_path(dest: &Path) -> PathBuf {
    let tmp_name = format!(
        ".openhuman-update-{}.tmp",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );
    dest.parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(tmp_name)
}

/// Make a file executable (Unix) and rename it atomically to `dest`. On rename
/// failure the temp file is best-effort cleaned up.
fn finalize_executable(tmp: &Path, dest: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(tmp, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("set permissions: {e}"))?;
    }
    std::fs::rename(tmp, dest).map_err(|e| {
        let _ = std::fs::remove_file(tmp);
        format!("rename staged binary: {e}")
    })
}

/// Extract the inner core binary from a downloaded archive into `dest`.
///
/// The archive is expected to contain a single file named `openhuman-core`
/// (or `openhuman-core.exe`) at the root — matching the layout produced by
/// the release workflow. `dest` must be the final binary path (with the
/// platform-triple-suffixed name `default_core_bin` looks for).
fn extract_archive(archive_path: &Path, dest: &Path) -> Result<(), String> {
    let inner_name = if cfg!(windows) {
        "openhuman-core.exe"
    } else {
        "openhuman-core"
    };

    let archive_name = archive_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    let file =
        std::fs::File::open(archive_path).map_err(|e| format!("open archive: {e}"))?;

    if archive_name.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("read zip: {e}"))?;
        let mut entry = zip
            .by_name(inner_name)
            .map_err(|e| format!("zip entry '{inner_name}' missing: {e}"))?;
        let tmp = unique_tmp_path(dest);
        {
            let mut out =
                std::fs::File::create(&tmp).map_err(|e| format!("create temp: {e}"))?;
            std::io::copy(&mut entry, &mut out).map_err(|e| format!("extract zip: {e}"))?;
            out.flush().map_err(|e| format!("flush extracted: {e}"))?;
        }
        finalize_executable(&tmp, dest)?;
        return Ok(());
    }

    // Default: tar.gz / tgz
    let gz = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(gz);
    let entries = tar
        .entries()
        .map_err(|e| format!("read tar entries: {e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("tar entry: {e}"))?;
        let entry_path = entry.path().map_err(|e| format!("entry path: {e}"))?;
        let matches = entry_path
            .file_name()
            .and_then(|s| s.to_str())
            .map(|n| n == inner_name)
            .unwrap_or(false);
        if !matches {
            continue;
        }
        let tmp = unique_tmp_path(dest);
        {
            let mut out =
                std::fs::File::create(&tmp).map_err(|e| format!("create temp: {e}"))?;
            std::io::copy(&mut entry, &mut out).map_err(|e| format!("extract tar: {e}"))?;
            out.flush().map_err(|e| format!("flush extracted: {e}"))?;
        }
        finalize_executable(&tmp, dest)?;
        return Ok(());
    }

    Err(format!(
        "archive {} contained no entry named '{inner_name}'",
        archive_path.display()
    ))
}

/// Download `url` to `dest` atomically. Used for both raw binaries (legacy
/// release format) and archive files (current format — caller then extracts).
async fn download_to_file(url: &str, dest: &Path) -> Result<(), String> {
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

    let tmp = unique_tmp_path(dest);
    {
        let mut file = std::fs::File::create(&tmp).map_err(|e| format!("create temp file: {e}"))?;
        file.write_all(&bytes)
            .map_err(|e| format!("write temp file: {e}"))?;
        file.flush().map_err(|e| format!("flush temp file: {e}"))?;
    }

    // Move into place. Caller is responsible for marking executable when the
    // payload is a raw binary; archives stay non-executable on disk.
    std::fs::rename(&tmp, dest).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("rename downloaded file: {e}")
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
    app: Option<tauri::AppHandle<crate::AppRuntime>>,
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

    // Where the downloaded asset lands (named after the release asset).
    let download_dest = staging_dir
        .as_ref()
        .map(|d| d.join(&asset.name))
        .unwrap_or_else(|| PathBuf::from(&asset.name));
    // Final binary path that `default_core_bin` will pick up next launch.
    let binary_dest = staging_dir
        .as_ref()
        .map(|d| d.join(staged_binary_name()))
        .unwrap_or_else(|| PathBuf::from(staged_binary_name()));
    let asset_is_archive = is_archive_asset(&asset.name);

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

        // Download the asset.
        download_to_file(&asset.browser_download_url, &download_dest).await?;
        log::info!(
            "[core-update] downloaded asset to {}",
            download_dest.display()
        );

        if asset_is_archive {
            // Extract the inner `openhuman-core` binary into the staged location.
            extract_archive(&download_dest, &binary_dest)?;
            log::info!(
                "[core-update] extracted core binary to {}",
                binary_dest.display()
            );
            // Best-effort cleanup of the archive (don't fail the update if this fails).
            if let Err(e) = std::fs::remove_file(&download_dest) {
                log::warn!(
                    "[core-update] could not remove archive {}: {e}",
                    download_dest.display()
                );
            }
        } else {
            // Legacy raw-binary asset: rename into the canonical staged path
            // and mark executable.
            if download_dest != binary_dest {
                std::fs::rename(&download_dest, &binary_dest)
                    .map_err(|e| format!("rename legacy binary: {e}"))?;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&binary_dest, std::fs::Permissions::from_mode(0o755))
                    .map_err(|e| format!("set permissions on staged binary: {e}"))?;
            }
            log::info!(
                "[core-update] staged legacy raw binary at {}",
                binary_dest.display()
            );
        }

        // Point the handle at the new binary so ensure_running launches it.
        handle.set_core_bin(binary_dest).await;

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

fn emit_event(app: &Option<tauri::AppHandle<crate::AppRuntime>>, event: &str, payload: &str) {
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

    fn asset(name: &str) -> GitHubAsset {
        GitHubAsset {
            name: name.to_string(),
            browser_download_url: format!("https://example.test/{name}"),
        }
    }

    #[test]
    fn find_platform_asset_matches_versioned_archive() {
        let triple = platform_triple();
        let archive_ext = if cfg!(windows) { "zip" } else { "tar.gz" };
        let archive_name = format!("openhuman-core-0.52.26-{triple}.{archive_ext}");
        let assets = vec![
            asset("latest.json"),
            asset(&format!("{archive_name}.sha256")),
            asset(&archive_name),
            asset(&format!("{archive_name}.sig")),
            asset(&format!("OpenHuman_0.52.26_{triple}.app.tar.gz")),
        ];
        let m = find_platform_asset(&assets).expect("should match versioned archive");
        assert_eq!(m.name, archive_name);
    }

    #[test]
    fn find_platform_asset_falls_back_to_legacy_raw_binary() {
        let triple = platform_triple();
        let legacy_name = if cfg!(windows) {
            format!("openhuman-core-{triple}.exe")
        } else {
            format!("openhuman-core-{triple}")
        };
        let assets = vec![asset("latest.json"), asset(&legacy_name)];
        let m = find_platform_asset(&assets).expect("legacy raw binary should match");
        assert_eq!(m.name, legacy_name);
    }

    #[test]
    fn find_platform_asset_skips_signatures_and_checksums() {
        let triple = platform_triple();
        let archive_ext = if cfg!(windows) { "zip" } else { "tar.gz" };
        let assets = vec![
            asset(&format!("openhuman-core-0.52.26-{triple}.{archive_ext}.sha256")),
            asset(&format!("openhuman-core-0.52.26-{triple}.{archive_ext}.sig")),
        ];
        assert!(find_platform_asset(&assets).is_none());
    }

    #[test]
    fn is_archive_asset_recognises_known_extensions() {
        assert!(is_archive_asset("openhuman-core-0.52.26-aarch64-apple-darwin.tar.gz"));
        assert!(is_archive_asset("openhuman-core-0.52.26-x86_64-pc-windows-msvc.zip"));
        assert!(is_archive_asset("foo.tgz"));
        assert!(!is_archive_asset("openhuman-core-aarch64-apple-darwin"));
        assert!(!is_archive_asset("openhuman-core.exe"));
    }

    #[test]
    fn extract_archive_pulls_inner_binary_from_targz() {
        use std::io::Read;

        let dir = std::env::temp_dir().join(format!(
            "oh-extract-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        // Build an in-memory tar.gz with a single `openhuman-core` (or .exe) entry.
        let inner = if cfg!(windows) {
            "openhuman-core.exe"
        } else {
            "openhuman-core"
        };
        let payload = b"#!fake-binary-bytes";

        let archive_path = dir.join("test.tar.gz");
        {
            let tar_gz = std::fs::File::create(&archive_path).unwrap();
            let enc = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
            let mut builder = tar::Builder::new(enc);
            let mut header = tar::Header::new_gnu();
            header.set_size(payload.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, inner, &payload[..])
                .unwrap();
            builder.into_inner().unwrap().finish().unwrap();
        }

        let dest = dir.join("openhuman-core-staged");
        extract_archive(&archive_path, &dest).expect("extract should succeed");

        let mut got = Vec::new();
        std::fs::File::open(&dest)
            .unwrap()
            .read_to_end(&mut got)
            .unwrap();
        assert_eq!(got, payload);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn outdated_detection() {
        assert!(is_outdated("0.49.17", "0.51.8"));
        assert!(is_outdated("0.50.0", "0.51.0"));
        assert!(!is_outdated("0.51.8", "0.51.8"));
        assert!(!is_outdated("0.52.0", "0.51.8"));
        assert!(!is_outdated("1.0.0", "0.51.8"));
    }
}
