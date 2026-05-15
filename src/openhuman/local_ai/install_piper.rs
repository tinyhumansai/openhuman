//! Piper installer — downloads the platform-specific Piper binary
//! archive and the bundled `en_US-lessac-medium` voice (`.onnx` +
//! `.onnx.json` sidecar) into the workspace.
//!
//! Voice IDs other than the bundled default are intentionally out of
//! scope; the VoicePanel exposes a free-text `tts_voice_id` input so
//! advanced users can manually drop in additional `.onnx` files alongside
//! the bundled one (see Voice TTS factory docs).

use std::io::Read;
use std::path::PathBuf;

use crate::openhuman::config::Config;

use super::paths;
use super::voice_install_common::{
    download_to_file, read_status, write_status, VoiceInstallState, VoiceInstallStatus,
    ENGINE_PIPER,
};

const LOG_PREFIX: &str = "[voice-install:piper]";

/// Default voice id shipped with the installer. Matches
/// [`crate::openhuman::voice::factory::DEFAULT_PIPER_VOICE`].
pub const DEFAULT_PIPER_VOICE: &str = "en_US-lessac-medium";

/// Minimum bytes for the Piper release archive. The smallest historical
/// build is ~7 MB; below 1 MB is almost certainly an error response.
const MIN_BINARY_ARCHIVE_BYTES: u64 = 1024 * 1024;

/// Minimum bytes for the voice `.onnx` model. `en_US-lessac-medium.onnx`
/// is ~60 MB; allow some slack for CDN compression differences.
const MIN_VOICE_BYTES: u64 = 30 * 1024 * 1024;

/// Minimum bytes for the `.onnx.json` sidecar. The file is human-readable
/// JSON, typically a few KB; anything below 256 bytes is almost certainly
/// a 404 HTML response masquerading as JSON.
const MIN_VOICE_JSON_BYTES: u64 = 256;

/// Result of resolving the Piper binary archive URL for the host OS.
struct BinaryAsset {
    url: String,
    /// Archive shape — drives the extraction strategy.
    kind: ArchiveKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArchiveKind {
    Zip,
    TarGz,
}

/// Per-OS Piper release asset URL. The Piper project publishes one
/// archive per OS/architecture under the `latest` release alias. Names
/// have been stable across recent releases.
fn binary_download_asset() -> Option<BinaryAsset> {
    let base = "https://github.com/rhasspy/piper/releases/latest/download";
    if cfg!(target_os = "windows") {
        return Some(BinaryAsset {
            url: format!("{base}/piper_windows_amd64.zip"),
            kind: ArchiveKind::Zip,
        });
    }
    if cfg!(target_os = "macos") {
        // Two assets exist (`piper_macos_x64.tar.gz` and
        // `piper_macos_aarch64.tar.gz`). Pick based on the host arch.
        let arch = std::env::consts::ARCH;
        let suffix = match arch {
            "aarch64" | "arm64" => "macos_aarch64",
            _ => "macos_x64",
        };
        return Some(BinaryAsset {
            url: format!("{base}/piper_{suffix}.tar.gz"),
            kind: ArchiveKind::TarGz,
        });
    }
    if cfg!(target_os = "linux") {
        let arch = std::env::consts::ARCH;
        let suffix = match arch {
            "aarch64" | "arm64" => "linux_aarch64",
            "armv7" | "arm" => "linux_armv7",
            _ => "linux_x86_64",
        };
        return Some(BinaryAsset {
            url: format!("{base}/piper_{suffix}.tar.gz"),
            kind: ArchiveKind::TarGz,
        });
    }
    None
}

/// Voice file URLs on HuggingFace. Returns `(onnx_url, onnx_json_url)`.
fn voice_download_urls(voice_id: &str) -> (String, String) {
    // The Piper voices repo uses the structure:
    //   en/en_US/lessac/medium/en_US-lessac-medium.onnx
    //   en/en_US/lessac/medium/en_US-lessac-medium.onnx.json
    // We only support the bundled default — multi-voice support is
    // tracked separately. The path components mirror the voice id.
    let (lang_short, locale, name, quality) = decode_voice_id(voice_id);
    let base = format!(
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/{lang_short}/{locale}/{name}/{quality}"
    );
    let stem = format!("{locale}-{name}-{quality}");
    (
        format!("{base}/{stem}.onnx"),
        format!("{base}/{stem}.onnx.json"),
    )
}

/// Decompose `en_US-lessac-medium` into its repo-path pieces.
///
/// Returns `(short_lang, locale, voice_name, quality)`.
fn decode_voice_id(voice_id: &str) -> (String, String, String, String) {
    // Fall back to the bundled default if the id is malformed — the
    // installer should never panic on user-typed input.
    let trimmed = voice_id.trim();
    let id = if trimmed.is_empty() {
        DEFAULT_PIPER_VOICE
    } else {
        trimmed
    };
    let parts: Vec<&str> = id.split('-').collect();
    if parts.len() < 3 {
        // Reuse the default decomposition on any malformed input so the
        // download URL is still well-formed (the install will fail at
        // size validation if the file doesn't exist upstream).
        return (
            "en".to_string(),
            "en_US".to_string(),
            "lessac".to_string(),
            "medium".to_string(),
        );
    }
    let locale = parts[0].to_string();
    let name = parts[1].to_string();
    let quality = parts[2..].join("-");
    let short_lang = locale.split('_').next().unwrap_or("en").to_string();
    (short_lang, locale, name, quality)
}

/// Convenience: read the current installer status snapshot, falling back
/// to "installed" when on-disk artifacts pass validation.
pub fn status(config: &Config) -> VoiceInstallStatus {
    let mut snapshot = read_status(ENGINE_PIPER);
    let configured_voice = crate::openhuman::local_ai::model_ids::effective_tts_voice_id(config);
    let configured_voice = configured_voice.trim_end_matches(".onnx").to_string();
    if matches!(snapshot.state, VoiceInstallState::Missing)
        && installed_artifacts_ok(config, &configured_voice)
    {
        snapshot.state = VoiceInstallState::Installed;
        snapshot.stage = Some("binary and voice present".to_string());
    }
    snapshot
}

fn installed_artifacts_ok(config: &Config, voice_id: &str) -> bool {
    // Check the SPECIFIC requested voice, not the hard-coded default.
    // Without this, switching voice via the dropdown would short-circuit
    // with "already installed" and never fetch the new `.onnx`.
    let voice_ok = paths::workspace_piper_voice_paths(config, voice_id)
        .map(|(onnx, json)| {
            let onnx_ok = std::fs::metadata(&onnx)
                .map(|m| m.is_file() && m.len() >= MIN_VOICE_BYTES)
                .unwrap_or(false);
            let json_ok = std::fs::metadata(&json)
                .map(|m| m.is_file() && m.len() >= MIN_VOICE_JSON_BYTES)
                .unwrap_or(false);
            log::debug!(
                "{LOG_PREFIX} install check onnx={} onnx_ok={} json={} json_ok={}",
                onnx.display(),
                onnx_ok,
                json.display(),
                json_ok
            );
            onnx_ok && json_ok
        })
        .unwrap_or(false);
    let binary_ok = paths::workspace_piper_binary_candidates(config)
        .iter()
        .any(|p| p.is_file());
    log::debug!(
        "{LOG_PREFIX} install check binary_ok={} voice_ok={}",
        binary_ok,
        voice_ok
    );
    binary_ok && voice_ok
}

/// Kick off (or re-kick) a Piper install. `force_reinstall = true`
/// removes any existing voice file first; otherwise an already-installed
/// engine returns immediately with a no-op success.
pub async fn install_piper(
    config: &Config,
    voice_id: Option<String>,
    force_reinstall: bool,
) -> Result<VoiceInstallStatus, String> {
    let voice = voice_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_PIPER_VOICE)
        .to_string();
    log::debug!("{LOG_PREFIX} install requested voice={voice} force_reinstall={force_reinstall}");

    if !force_reinstall && installed_artifacts_ok(config, &voice) {
        log::debug!("{LOG_PREFIX} short-circuit: artifacts already present");
        let snapshot = VoiceInstallStatus {
            engine: ENGINE_PIPER.to_string(),
            state: VoiceInstallState::Installed,
            progress: Some(100),
            downloaded_bytes: None,
            total_bytes: None,
            stage: Some("already installed".to_string()),
            error_detail: None,
        };
        write_status(snapshot.clone());
        return Ok(snapshot);
    }

    write_status(VoiceInstallStatus {
        engine: ENGINE_PIPER.to_string(),
        state: VoiceInstallState::Installing,
        progress: Some(0),
        downloaded_bytes: Some(0),
        total_bytes: None,
        stage: Some(format!("starting piper install ({voice})")),
        error_detail: None,
    });

    let result = run_install(config, &voice).await;
    match &result {
        Ok(()) => {
            let snapshot = VoiceInstallStatus {
                engine: ENGINE_PIPER.to_string(),
                state: VoiceInstallState::Installed,
                progress: Some(100),
                downloaded_bytes: None,
                total_bytes: None,
                stage: Some("install complete".to_string()),
                error_detail: None,
            };
            write_status(snapshot.clone());
            Ok(snapshot)
        }
        Err(msg) => {
            let snapshot = VoiceInstallStatus {
                engine: ENGINE_PIPER.to_string(),
                state: VoiceInstallState::Error,
                progress: None,
                downloaded_bytes: None,
                total_bytes: None,
                stage: None,
                error_detail: Some(msg.clone()),
            };
            write_status(snapshot.clone());
            Err(msg.clone())
        }
    }
}

async fn run_install(config: &Config, voice: &str) -> Result<(), String> {
    // 1) Voice files: `.onnx` (heavy) + `.onnx.json` (small sidecar).
    let (onnx_url, json_url) = voice_download_urls(voice);
    let (onnx_path, json_path) = paths::workspace_piper_voice_paths(config, voice)
        .ok_or_else(|| format!("{LOG_PREFIX} could not resolve voice paths for '{voice}'"))?;

    log::debug!("{LOG_PREFIX} downloading voice url={onnx_url}");
    update_stage(format!("downloading {voice}.onnx"));
    download_to_file(
        &onnx_url,
        &onnx_path,
        None,
        MIN_VOICE_BYTES,
        LOG_PREFIX,
        |downloaded, total| {
            let progress = total
                .filter(|t| *t > 0)
                .map(|t| ((downloaded * 100) / t).min(100) as u8);
            write_status(VoiceInstallStatus {
                engine: ENGINE_PIPER.to_string(),
                state: VoiceInstallState::Installing,
                progress,
                downloaded_bytes: Some(downloaded),
                total_bytes: total,
                stage: Some("downloading voice (.onnx)".to_string()),
                error_detail: None,
            });
        },
    )
    .await?;
    log::debug!("{LOG_PREFIX} voice .onnx staged at {}", onnx_path.display());

    log::debug!("{LOG_PREFIX} downloading voice json url={json_url}");
    update_stage(format!("downloading {voice}.onnx.json"));
    download_to_file(
        &json_url,
        &json_path,
        None,
        MIN_VOICE_JSON_BYTES,
        LOG_PREFIX,
        |downloaded, total| {
            let progress = total
                .filter(|t| *t > 0)
                .map(|t| ((downloaded * 100) / t).min(100) as u8);
            write_status(VoiceInstallStatus {
                engine: ENGINE_PIPER.to_string(),
                state: VoiceInstallState::Installing,
                progress,
                downloaded_bytes: Some(downloaded),
                total_bytes: total,
                stage: Some("downloading voice (.onnx.json)".to_string()),
                error_detail: None,
            });
        },
    )
    .await?;

    // 2) Binary archive.
    let asset = binary_download_asset()
        .ok_or_else(|| format!("{LOG_PREFIX} no piper binary release for this OS/arch"))?;
    let archive_name = asset
        .url
        .rsplit('/')
        .next()
        .unwrap_or("piper_archive")
        .to_string();
    let archive_path = paths::workspace_piper_dir(config).join(&archive_name);
    log::debug!("{LOG_PREFIX} downloading binary url={}", asset.url);
    update_stage("downloading piper binary".to_string());
    download_to_file(
        &asset.url,
        &archive_path,
        None,
        MIN_BINARY_ARCHIVE_BYTES,
        LOG_PREFIX,
        |downloaded, total| {
            let progress = total
                .filter(|t| *t > 0)
                .map(|t| ((downloaded * 100) / t).min(100) as u8);
            write_status(VoiceInstallStatus {
                engine: ENGINE_PIPER.to_string(),
                state: VoiceInstallState::Installing,
                progress,
                downloaded_bytes: Some(downloaded),
                total_bytes: total,
                stage: Some("downloading binary".to_string()),
                error_detail: None,
            });
        },
    )
    .await?;
    update_stage("extracting piper binary".to_string());
    let dest = paths::workspace_piper_dir(config);
    match asset.kind {
        ArchiveKind::Zip => extract_zip(&archive_path, &dest)?,
        ArchiveKind::TarGz => extract_tar_gz(&archive_path, &dest)?,
    }
    let _ = std::fs::remove_file(&archive_path);

    Ok(())
}

fn update_stage(stage: String) {
    let mut current = read_status(ENGINE_PIPER);
    current.stage = Some(stage);
    write_status(current);
}

fn extract_zip(zip_path: &std::path::Path, dest_dir: &std::path::Path) -> Result<(), String> {
    log::debug!(
        "{LOG_PREFIX} extract_zip {} -> {}",
        zip_path.display(),
        dest_dir.display()
    );
    let file = std::fs::File::open(zip_path).map_err(|e| format!("{LOG_PREFIX} open zip: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("{LOG_PREFIX} parse zip: {e}"))?;
    std::fs::create_dir_all(dest_dir).map_err(|e| format!("{LOG_PREFIX} mkdir dest: {e}"))?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("{LOG_PREFIX} zip entry {i}: {e}"))?;
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        let rel = rel.to_path_buf();
        let out_path = dest_dir.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)
                .map_err(|e| format!("{LOG_PREFIX} mkdir {}: {e}", out_path.display()))?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("{LOG_PREFIX} mkdir {}: {e}", parent.display()))?;
            }
            let mut out = std::fs::File::create(&out_path)
                .map_err(|e| format!("{LOG_PREFIX} create {}: {e}", out_path.display()))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| format!("{LOG_PREFIX} copy {}: {e}", out_path.display()))?;
        }
    }
    Ok(())
}

fn extract_tar_gz(archive: &std::path::Path, dest_dir: &std::path::Path) -> Result<(), String> {
    log::debug!(
        "{LOG_PREFIX} extract_tar_gz {} -> {}",
        archive.display(),
        dest_dir.display()
    );
    std::fs::create_dir_all(dest_dir).map_err(|e| format!("{LOG_PREFIX} mkdir dest: {e}"))?;
    let file =
        std::fs::File::open(archive).map_err(|e| format!("{LOG_PREFIX} open tar.gz: {e}"))?;
    // The Piper tarball is gzipped. The `flate2` crate is already a
    // transitive dep through `tar`; if it's not directly available we
    // would need to add it here. As of this writing the workspace uses
    // gzip-aware tar via the `flate2` dep that ships with `zip`'s
    // companion utilities — but the standard pattern in this codebase
    // is to shell out to `tar` so we don't grow the dep tree.
    //
    // To keep the installer self-contained without adding a new
    // workspace dep, decompress in-memory then hand the plain tar to
    // the `tar` crate. The Piper archive is only ~7 MB so a single
    // in-memory inflate is acceptable.
    let mut gz = std::io::BufReader::new(file);
    let mut compressed = Vec::new();
    gz.read_to_end(&mut compressed)
        .map_err(|e| format!("{LOG_PREFIX} read tar.gz: {e}"))?;
    let decompressed =
        inflate_gzip(&compressed).map_err(|e| format!("{LOG_PREFIX} inflate tar.gz: {e}"))?;
    let mut tar = tar::Archive::new(std::io::Cursor::new(decompressed));
    tar.unpack(dest_dir)
        .map_err(|e| format!("{LOG_PREFIX} unpack tar: {e}"))?;
    Ok(())
}

/// Inflate a gzip stream using the `flate2` crate that ships with `zip`'s
/// deflate feature. We re-export through the `zip` crate's surface to
/// avoid a direct flate2 dep declaration.
fn inflate_gzip(compressed: &[u8]) -> Result<Vec<u8>, String> {
    // `flate2` is pulled in transitively by `zip` with the `deflate`
    // feature. Use its public reader API directly.
    use flate2::read::GzDecoder;
    let mut decoder = GzDecoder::new(compressed);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|e| format!("gz decode: {e}"))?;
    Ok(out)
}

/// Return the workspace-installed Piper binary path if one exists. Used
/// by `paths::resolve_piper_binary` to prefer the workspace install over
/// `PIPER_BIN` / PATH.
pub(crate) fn find_workspace_piper_binary(config: &Config) -> Option<PathBuf> {
    let candidates = paths::workspace_piper_binary_candidates(config);
    for candidate in candidates {
        if candidate.is_file() {
            log::debug!(
                "{LOG_PREFIX} found workspace piper binary at {}",
                candidate.display()
            );
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::local_ai::voice_install_common::reset_status;

    fn temp_config() -> (tempfile::TempDir, Config) {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = Config {
            workspace_dir: dir.path().join("workspace"),
            config_path: dir.path().join("config.toml"),
            ..Config::default()
        };
        (dir, config)
    }

    #[test]
    fn decode_voice_id_splits_correctly() {
        assert_eq!(
            decode_voice_id("en_US-lessac-medium"),
            (
                "en".to_string(),
                "en_US".to_string(),
                "lessac".to_string(),
                "medium".to_string()
            )
        );
        assert_eq!(
            decode_voice_id("de_DE-thorsten-high"),
            (
                "de".to_string(),
                "de_DE".to_string(),
                "thorsten".to_string(),
                "high".to_string()
            )
        );
    }

    #[test]
    fn decode_voice_id_falls_back_for_garbage() {
        // Single-piece input is malformed → bundled default decomposition.
        let (lang, locale, name, quality) = decode_voice_id("garbage");
        assert_eq!(lang, "en");
        assert_eq!(locale, "en_US");
        assert_eq!(name, "lessac");
        assert_eq!(quality, "medium");

        let (_lang, _locale, _name, _quality) = decode_voice_id("");
        // Empty string also produces the bundled default — guarded above.
    }

    #[test]
    fn voice_download_urls_anchor_on_hf_bucket() {
        let (onnx, json) = voice_download_urls("en_US-lessac-medium");
        assert!(onnx.starts_with("https://huggingface.co/rhasspy/piper-voices/resolve/main/"));
        assert!(onnx.ends_with("en_US-lessac-medium.onnx"));
        assert!(json.ends_with("en_US-lessac-medium.onnx.json"));
    }

    #[test]
    fn binary_download_asset_picks_an_os_specific_url() {
        let asset = binary_download_asset();
        // On supported platforms we expect an asset; the test only runs
        // on the host so this is informative.
        if cfg!(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "linux"
        )) {
            let asset = asset.expect("supported platform should return an asset");
            assert!(asset.url.contains("piper"));
            assert!(asset
                .url
                .starts_with("https://github.com/rhasspy/piper/releases"));
            if cfg!(windows) {
                assert_eq!(asset.kind, ArchiveKind::Zip);
            } else {
                assert_eq!(asset.kind, ArchiveKind::TarGz);
            }
        } else {
            assert!(asset.is_none());
        }
    }

    /// Serialise tests that write into the shared `~/.openhuman/bin/piper/`
    /// directory; reuses the module-wide `local_ai_test_guard` so paths +
    /// install_whisper tests are serialised through the same lock.
    fn shared_install_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::openhuman::local_ai::local_ai_test_guard()
    }

    fn wipe_shared_install_dir(config: &Config) {
        let dir = paths::workspace_piper_dir(config);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn status_reports_missing_for_fresh_workspace() {
        let _g = shared_install_lock();
        reset_status(ENGINE_PIPER);
        let (_tmp, config) = temp_config();
        wipe_shared_install_dir(&config);
        let snapshot = status(&config);
        assert_eq!(snapshot.state, VoiceInstallState::Missing);
    }

    /// Build a `.onnx.json` payload big enough to pass the size floor.
    /// Real Piper sidecars are a few KB; the floor exists to reject 404
    /// HTML pages, so as long as we write past 256 bytes we mirror the
    /// production validator's accept set.
    fn synthetic_voice_json() -> Vec<u8> {
        let mut body = br#"{"audio":{"sample_rate":22050},"phoneme_id_map":{},"#.to_vec();
        // Pad to comfortably exceed the size floor without altering shape.
        body.extend_from_slice(br#""filler":""#);
        body.extend(std::iter::repeat_n(b'x', 512));
        body.extend_from_slice(br#""}"#);
        body
    }

    #[test]
    fn status_promotes_to_installed_when_voice_and_binary_present() {
        let _g = shared_install_lock();
        reset_status(ENGINE_PIPER);
        let (_tmp, config) = temp_config();
        wipe_shared_install_dir(&config);
        // Voice files.
        let (onnx, json) =
            paths::workspace_piper_voice_paths(&config, DEFAULT_PIPER_VOICE).expect("voice paths");
        std::fs::create_dir_all(onnx.parent().unwrap()).unwrap();
        std::fs::write(&onnx, vec![0u8; (MIN_VOICE_BYTES + 1024) as usize]).unwrap();
        std::fs::write(&json, synthetic_voice_json()).unwrap();
        // Binary.
        let bin_candidate = paths::workspace_piper_binary_candidates(&config)[0].clone();
        std::fs::create_dir_all(bin_candidate.parent().unwrap()).unwrap();
        std::fs::write(&bin_candidate, b"stub").unwrap();

        let snapshot = status(&config);
        assert_eq!(snapshot.state, VoiceInstallState::Installed);
        wipe_shared_install_dir(&config);
    }

    // Same rationale as install_whisper.rs: holding the sync mutex over
    // the install await is safe because the install path doesn't acquire
    // any other locks, and the guard's job is to keep filesystem writes
    // from racing with sibling tests.
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn install_short_circuits_when_already_installed() {
        let _g = shared_install_lock();
        reset_status(ENGINE_PIPER);
        let (_tmp, config) = temp_config();
        wipe_shared_install_dir(&config);
        let (onnx, json) =
            paths::workspace_piper_voice_paths(&config, DEFAULT_PIPER_VOICE).expect("voice paths");
        std::fs::create_dir_all(onnx.parent().unwrap()).unwrap();
        std::fs::write(&onnx, vec![0u8; (MIN_VOICE_BYTES + 1024) as usize]).unwrap();
        std::fs::write(&json, synthetic_voice_json()).unwrap();
        let bin_candidate = paths::workspace_piper_binary_candidates(&config)[0].clone();
        std::fs::create_dir_all(bin_candidate.parent().unwrap()).unwrap();
        std::fs::write(&bin_candidate, b"stub").unwrap();

        let result = install_piper(&config, None, false).await;
        assert!(result.is_ok(), "short-circuit must succeed: {result:?}");
        let snap = result.unwrap();
        assert_eq!(snap.state, VoiceInstallState::Installed);
        wipe_shared_install_dir(&config);
    }

    #[test]
    fn find_workspace_piper_binary_returns_path_when_present() {
        let _g = shared_install_lock();
        let (_tmp, config) = temp_config();
        wipe_shared_install_dir(&config);
        let target = paths::workspace_piper_binary_candidates(&config)[0].clone();
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"stub").unwrap();
        let found = find_workspace_piper_binary(&config).expect("should find binary");
        assert_eq!(found, target);
        wipe_shared_install_dir(&config);
    }

    #[test]
    fn find_workspace_piper_binary_returns_none_without_install() {
        let _g = shared_install_lock();
        let (_tmp, config) = temp_config();
        wipe_shared_install_dir(&config);
        assert!(find_workspace_piper_binary(&config).is_none());
    }
}
