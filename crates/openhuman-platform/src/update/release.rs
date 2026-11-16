//! Release discovery and staging: check GitHub Releases for a newer build of
//! the binary an [`UpdateSource`] names, and download + stage it for the
//! caller to swap in.

use std::io::Write;
use std::path::{Path, PathBuf};

use super::download;
use std::sync::{Arc, Mutex, OnceLock};

/// Locks staging mutations by destination so concurrent update requests cannot
/// truncate shared temporary files or replace one another's completed binary.
fn staging_lock(dest: &std::path::Path) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<std::collections::HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> =
        OnceLock::new();
    let locks = LOCKS.get_or_init(Default::default);
    let mut locks = locks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Arc::clone(
        locks
            .entry(dest.to_path_buf())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
    )
}

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

use super::types::{
    GitHubAsset, GitHubRelease, StagedUpdate, UpdateInfo, UpdateReporter, UpdateSource,
};

/// Origin of the release-metadata API. Not configurable at runtime — see
/// `check_available_with_base_url`.
const GITHUB_API_BASE: &str = "https://api.github.com";

/// `User-Agent` for release requests: `<asset_prefix>-updater`.
fn user_agent(source: &UpdateSource) -> String {
    format!("{}-updater", source.asset_prefix)
}

/// The longest prefix of `s` that is at most `max_bytes` long and ends on a
/// UTF-8 character boundary.
fn utf8_prefix(s: &str, max_bytes: usize) -> &str {
    let mut end = max_bytes.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
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

/// True if the asset name is an archive whose inner binary must be extracted,
/// rather than a raw executable that can be staged as downloaded.
fn is_archive_asset(name: &str) -> bool {
    name.ends_with(".tar.gz") || name.ends_with(".tgz") || name.ends_with(".zip")
}

/// Find the right asset for this platform from a list of release assets.
///
/// The release workflow publishes the core as a versioned archive,
/// `<prefix>-<version>-<triple>.tar.gz` (`.zip` on Windows) — the version
/// sits between the prefix and the triple. Matching on `<prefix>-{triple}`
/// alone therefore never hits, on any platform, which is what left every update
/// check reporting `asset=(none)` (#6766). #908 fixed this once; the matcher was
/// reintroduced in its pre-#908 form when this module moved into the core, so the
/// shape below is restored from that fix.
///
/// The raw-binary form is kept as a fallback for older releases.
fn find_platform_asset<'a>(
    source: &UpdateSource,
    assets: &'a [GitHubAsset],
) -> Option<&'a GitHubAsset> {
    let triple = platform_triple();
    let archive_ext = if cfg!(windows) { ".zip" } else { ".tar.gz" };
    let prefix = format!("{}-", source.asset_prefix);

    log::debug!(
        "[update] looking for an '{}*{}{}' asset among {} assets",
        prefix,
        triple,
        archive_ext,
        assets.len()
    );

    let archive_match = assets.iter().find(|a| {
        a.name.starts_with(&prefix)
            && a.name.contains(triple)
            && a.name.ends_with(archive_ext)
            // The release publishes a checksum and a signature beside each
            // archive (`….tar.gz.sha256`, `….tar.gz.sig`). Both share the
            // prefix and the triple, so exclude them explicitly.
            && !a.name.ends_with(".sha256")
            && !a.name.ends_with(".sig")
    });
    if archive_match.is_some() {
        return archive_match;
    }

    // Legacy raw-binary releases.
    let legacy = format!("{prefix}{triple}");
    let legacy_exe = format!("{legacy}.exe");
    assets
        .iter()
        .find(|a| a.name == legacy || a.name == legacy_exe)
}

/// Name the staged executable is written under.
///
/// The archive carries the binary as plain `<prefix>` (`.exe` on Windows),
/// which is the name the shell restarts, so the extracted file keeps it rather
/// than inheriting the archive's `.tar.gz` name.
fn staged_binary_name(source: &UpdateSource) -> String {
    if cfg!(windows) {
        format!("{}.exe", source.asset_prefix)
    } else {
        source.asset_prefix.to_string()
    }
}

fn staged_binary_staging_name(source: &UpdateSource) -> String {
    if cfg!(windows) {
        format!("{}-staged.exe", source.asset_prefix)
    } else {
        format!("{}.staged", source.asset_prefix)
    }
}

fn staged_asset_path(
    source: &UpdateSource,
    dir: &Path,
    asset_name: &str,
    is_archive: bool,
) -> PathBuf {
    if is_archive || asset_name == staged_binary_name(source) {
        dir.join(staged_binary_staging_name(source))
    } else {
        dir.join(asset_name)
    }
}

/// Make `tmp` executable and move it onto `dest` atomically, cleaning up `tmp`
/// if the rename fails.
fn finalize_executable(tmp: &Path, dest: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        std::fs::set_permissions(tmp, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("failed to set executable permission: {e}"))?;
    }
    std::fs::rename(tmp, dest).map_err(|e| {
        let _ = std::fs::remove_file(tmp);
        format!("failed to move update to {}: {e}", dest.display())
    })
}

/// Extract the inner `<prefix>` executable from a downloaded archive to
/// `dest`.
///
/// Without this the archive itself was written to the staging path and marked
/// executable, so a restart would try to exec a tarball. Restored from #908,
/// which added it for exactly that reason.
fn extract_core_binary(
    source: &UpdateSource,
    archive_path: &Path,
    dest: &Path,
    is_zip: bool,
) -> Result<(), String> {
    let inner_name = staged_binary_name(source);
    let inner_name = inner_name.as_str();
    let tmp_path = dest.with_extension("staging-tmp");

    let file = std::fs::File::open(archive_path).map_err(|e| format!("open archive: {e}"))?;

    if is_zip {
        let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("read zip: {e}"))?;
        let mut entry = zip
            .by_name(inner_name)
            .map_err(|e| format!("archive has no '{inner_name}' entry: {e}"))?;
        {
            let mut out = std::fs::File::create(&tmp_path)
                .map_err(|e| format!("create staging temp: {e}"))?;
            std::io::copy(&mut entry, &mut out).map_err(|e| format!("extract zip entry: {e}"))?;
            out.flush()
                .map_err(|e| format!("flush extracted binary: {e}"))?;
        }
        return finalize_executable(&tmp_path, dest);
    }

    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|e| format!("read tar entries: {e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("read tar entry: {e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("read tar entry path: {e}"))?;
        let is_core = path
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|name| name == inner_name);
        if !is_core {
            continue;
        }
        {
            let mut out = std::fs::File::create(&tmp_path)
                .map_err(|e| format!("create staging temp: {e}"))?;
            std::io::copy(&mut entry, &mut out).map_err(|e| format!("extract tar entry: {e}"))?;
            out.flush()
                .map_err(|e| format!("flush extracted binary: {e}"))?;
        }
        return finalize_executable(&tmp_path, dest);
    }

    Err(format!("archive has no '{inner_name}' entry"))
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

/// Check `source`'s GitHub Releases for a version newer than `current`.
pub async fn check_available(
    source: &UpdateSource,
    current: &str,
    reporter: &dyn UpdateReporter,
) -> Result<UpdateInfo, String> {
    check_available_with_base_url(source, current, reporter, GITHUB_API_BASE).await
}

/// `check_available`, with the API origin injected.
///
/// Deliberately **private**, and deliberately not an env var. The core's
/// `update::ops::validate_download_url` pins asset downloads to the GitHub host
/// allowlist on purpose; a runtime-overridable release endpoint would be a
/// self-update redirection primitive on the highest-trust path in the product.
/// The only caller besides `check_available` is the unit suite, which points it
/// at a local mock.
async fn check_available_with_base_url(
    source: &UpdateSource,
    current: &str,
    reporter: &dyn UpdateReporter,
    base_url: &str,
) -> Result<UpdateInfo, String> {
    log::info!(
        "[update] checking for updates — current version: {}",
        current
    );

    let url = format!(
        "{base_url}/repos/{}/{}/releases/latest",
        source.owner, source.repo
    );

    let client = reqwest::Client::builder()
        .user_agent(user_agent(source))
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;

    let response = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| {
            let msg = format!("failed to fetch latest release: {e}");
            if is_transport_network_failure(&e) || reporter.is_transient_message(&msg) {
                // OPENHUMAN-TAURI-2F: reqwest's transport-level failure fires
                // before any HTTP status when DNS / TCP / TLS handshake fails,
                // or the user's ISP / firewall blocks api.github.com. No
                // status, no trace, no payload — Sentry has no signal to act
                // on, and every scheduled poll generates another noisy event.
                // Log a warn so it shows up in local diagnostics and the next
                // tick can retry, without paging.
                tracing::warn!(
                    domain = "update",
                    operation = "check_releases",
                    failure = "transport",
                    "[observability] update.check_releases skipped transient updater transport failure: {msg}"
                );
            } else {
                reporter.report(msg.as_str(), "check_releases", &[("failure", "transport")]);
            }
            msg
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let status_str = status.as_u16().to_string();
        let body = response.text().await.unwrap_or_else(|_| "(no body)".into());
        log::warn!(
            "[update] GitHub API returned {}: {}",
            status,
            utf8_prefix(&body, 200)
        );
        let msg = format!("GitHub API error: {status}");
        if reporter.is_transient_http_status(status.as_u16()) {
            tracing::warn!(
                domain = "update",
                operation = "check_releases",
                failure = "non_2xx",
                status = status_str.as_str(),
                "[observability] update.check_releases skipped transient updater HTTP response: {msg}"
            );
        } else {
            reporter.report(
                msg.as_str(),
                "check_releases",
                &[("status", status_str.as_str()), ("failure", "non_2xx")],
            );
        }
        return Err(msg);
    }

    let release: GitHubRelease = response
        .json()
        .await
        .map_err(|e| format!("failed to parse release JSON: {e}"))?;

    let latest_version = release.tag_name.trim_start_matches('v').to_string();
    let update_available = is_newer(&latest_version, current);
    let platform_asset = find_platform_asset(source, &release.assets);

    if update_available && platform_asset.is_none() {
        if cfg!(target_os = "linux") {
            // Releases publish a core archive for Linux, so its absence is a
            // broken release the normal updater cannot apply.
            let message = format!(
                "update {latest_version} is available, but no core asset was found for {}",
                platform_triple()
            );
            log::error!("[update] {message}");
            reporter.report(
                &message,
                "check_releases",
                &[("failure", "missing_platform_asset")],
            );
            return Err(message);
        }
        // Expected off Linux, not a failure (Sentry TAURI-RUST-122R/122S/13B8/13B9):
        // macOS and Windows update through the Tauri updater's installers, not
        // a core archive. Report the update with no download url; `update_run`
        // answers that with `missing_asset_result`.
        log::warn!(
            "[update] update {latest_version} is available, but no core asset was published for {}; reporting it without a download",
            platform_triple()
        );
    }

    let info = UpdateInfo {
        latest_version,
        current_version: current.to_string(),
        update_available,
        download_url: platform_asset.map(|a| a.browser_download_url.clone()),
        asset_name: platform_asset.map(|a| a.name.clone()),
        asset_size: platform_asset.map(|a| a.size),
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
/// The asset's `.sha256` sidecar is fetched first, then the asset is streamed
/// to a temp file and moved to the staging path only if its digest matches.
/// The caller is responsible for killing the old process and restarting with
/// the new binary.
///
/// `staging_dir` — directory where the new binary should be placed (e.g.
/// the `binaries/` dir next to the Tauri app, or the Resources dir).
/// If `None`, uses the directory of the currently running executable.
///
/// `expected_size` — the asset size the release declared. The body must be
/// exactly this long. If `None`, the body is capped at `MAX_UPDATE_BYTES`.
pub async fn download_and_stage(
    source: &UpdateSource,
    download_url: &str,
    asset_name: &str,
    staging_dir: Option<PathBuf>,
    expected_size: Option<u64>,
    reporter: &dyn UpdateReporter,
) -> Result<StagedUpdate, String> {
    log::info!(
        "[update] downloading update from {} (asset: {})",
        download_url,
        asset_name
    );

    let client = reqwest::Client::builder()
        .user_agent(user_agent(source))
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;

    // Fail closed before the archive is fetched: an update without a
    // checksum is never staged.
    let expected_sha256 = download::fetch_expected_sha256(&client, download_url)
        .await
        .inspect_err(|e| log::error!("[update] checksum lookup failed asset={asset_name}: {e}"))?;

    let response = client.get(download_url).send().await.map_err(|e| {
        let msg = format!("failed to download update: {e}");
        if is_transport_network_failure(&e) {
            // Same transport-level shape as `check_releases` above
            // (OPENHUMAN-TAURI-2F) — DNS / TCP / TLS / firewall failure that
            // carries no actionable Sentry signal. The user-visible error is
            // still returned; Sentry just doesn't get spammed.
            log::warn!(
                "[update] download skipped transport-level failure asset={asset_name}: {msg}"
            );
        } else {
            reporter.report(
                msg.as_str(),
                "download",
                &[("asset", asset_name), ("failure", "transport")],
            );
        }
        msg
    })?;

    if !response.status().is_success() {
        let status = response.status();
        let status_str = status.as_u16().to_string();
        let msg = format!("download failed with status {}", status);
        if reporter.is_transient_http_status(status.as_u16()) {
            tracing::warn!(
                domain = "update",
                operation = "download",
                failure = "non_2xx",
                status = status_str.as_str(),
                asset = asset_name,
                "[observability] update.download skipped transient updater HTTP response: {msg}"
            );
        } else {
            reporter.report(
                msg.as_str(),
                "download",
                &[
                    ("asset", asset_name),
                    ("status", status_str.as_str()),
                    ("failure", "non_2xx"),
                ],
            );
        }
        return Err(msg);
    }

    let max_bytes = expected_size.unwrap_or(download::MAX_UPDATE_BYTES);
    download::check_declared_length(&response, max_bytes)
        .inspect_err(|e| report_rejected_download(reporter, e, asset_name, "oversize"))?;

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

    // An archive stages as the executable it contains, not as itself: marking a
    // `.tar.gz` 0755 and restarting into it cannot work (#6766).
    let is_archive = is_archive_asset(asset_name);
    // Legacy raw-binary assets can have the same name as the running executable.
    let staged_path = staged_asset_path(source, &dir, asset_name, is_archive);
    // Keep the lock through download-temp creation, extraction, and final rename:
    // those paths are shared by requests targeting the same staged executable.
    let destination_lock = staging_lock(&staged_path);
    let _destination_guard = destination_lock.lock().await;

    // Stream to a temp file first, then rename for atomicity.
    let tmp_path = dir.join(format!(".{asset_name}.tmp"));
    let mut file = std::fs::File::create(&tmp_path)
        .map_err(|e| format!("failed to create the download temp file for {asset_name}: {e}"))?;
    let streamed = download::stream_to_file(response, &mut file, max_bytes).await;
    let refusal = match streamed {
        Ok((written, digest)) => {
            log::info!("[update] downloaded {written} bytes");
            download::verify(written, &digest, max_bytes, expected_size, &expected_sha256).err()
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e);
        }
    };
    if let Some(e) = refusal {
        let _ = std::fs::remove_file(&tmp_path);
        report_rejected_download(reporter, &e, asset_name, "verification");
        return Err(e);
    }

    if is_archive {
        let extracted = extract_core_binary(
            source,
            &tmp_path,
            &staged_path,
            asset_name.ends_with(".zip"),
        );
        // The archive is scratch either way.
        let _ = std::fs::remove_file(&tmp_path);
        extracted?;
    } else {
        finalize_executable(&tmp_path, &staged_path)?;
    }

    log::info!("[update] staged update binary at {}", staged_path.display());

    Ok(StagedUpdate { staged_path })
}

/// Report an update body refused by the size or checksum check. These are not
/// transport noise: a mismatch on the restart path is worth a Sentry event.
fn report_rejected_download(
    reporter: &dyn UpdateReporter,
    message: &str,
    asset_name: &str,
    failure: &str,
) {
    log::error!("[update] refusing update asset={asset_name}: {message}");
    reporter.report(
        message,
        "download",
        &[("asset", asset_name), ("failure", failure)],
    );
}

/// Classify a reqwest failure as a user-environment transport problem (DNS
/// resolution, TCP connect refused/reset, TLS handshake, request timeout,
/// or any other `reqwest` "request sending" failure that fires before an
/// HTTP response is received).
///
/// These conditions have no actionable Sentry signal — no status, no trace,
/// no payload — and routinely show up when the user is offline, on a flaky
/// VPN, behind a captive portal, or in a region where api.github.com is
/// blocked. Filtering them at the call site keeps Sentry focused on real
/// regressions while leaving local `warn`-level diagnostics intact.
///
/// Reqwest 0.12's `is_request()` is the catch-all for `Kind::Request`
/// failures emitted by the underlying transport; `is_connect()` and
/// `is_timeout()` cover narrower buckets that may not always set
/// `is_request()`. Together they describe "the request could not be sent".
fn is_transport_network_failure(err: &reqwest::Error) -> bool {
    err.is_connect() || err.is_timeout() || err.is_request()
}

#[cfg(test)]
#[path = "release_tests.rs"]
mod tests;
