//! Whisper installer — downloads the GGML model file (and best-effort
//! `whisper-cli` binary when an upstream release asset exists for the
//! target OS) into the workspace.
//!
//! ## Scope notes
//!
//! The whisper.cpp project doesn't ship pre-built binaries with a
//! perfectly consistent naming scheme across OSes — Linux distros are
//! typically built from source, macOS arrives via Homebrew (`brew install
//! whisper-cpp`), and only Windows has a stable `.zip` asset on the
//! GitHub release page (`whisper-bin-x64.zip`). Our strategy:
//!
//! 1. **Always** download the GGML model file. This is the heavy artifact
//!    (1.6 GB for `large-v3-turbo`) and the one the local STT factory
//!    cannot run without.
//! 2. **Windows**: download the `whisper-bin-x64.zip` Windows release
//!    asset, unzip into the workspace, and surface the binary path.
//! 3. **macOS / Linux**: skip the binary fetch and leave a clear
//!    diagnostic note telling the user to install `whisper-cli` via
//!    their package manager. The model file is still ready for use the
//!    moment a binary lands on PATH.
//!
//! Per-engine progress is reported via the shared
//! [`crate::openhuman::local_ai::voice_install_common`] status table so
//! the renderer can poll one RPC for state across both Whisper and Piper.

use std::path::PathBuf;

use crate::openhuman::config::Config;

use super::paths;
use super::voice_install_common::{
    download_to_file, read_status, write_status, VoiceInstallState, VoiceInstallStatus,
    ENGINE_WHISPER,
};

const LOG_PREFIX: &str = "[voice-install:whisper]";

/// Default model size when the caller omits one. Matches
/// [`crate::openhuman::voice::factory::DEFAULT_WHISPER_MODEL`].
pub const DEFAULT_WHISPER_MODEL_SIZE: &str = "medium";

/// Minimum bytes for the smallest model (tiny is ~39 MB on disk; allow
/// some slack for HF mirror compression differences). Anything below this
/// is almost certainly an HTML error page from the CDN — refuse to
/// finalize the file.
const MIN_MODEL_BYTES: u64 = 30 * 1024 * 1024;

/// Minimum bytes for the Windows whisper-cli release zip. The smallest
/// historical build is ~5 MB; anything tinier is an error page.
const MIN_BINARY_ZIP_BYTES: u64 = 1024 * 1024;

/// Resolve the human-readable size token (`tiny`, `base`, `small`,
/// `medium`, `large-v3-turbo`) into the GGML filename used by
/// whisper.cpp's HuggingFace bucket.
fn ggml_filename(size: &str) -> String {
    // The bucket convention is `ggml-<size>.bin`. Variants exist for
    // quantization (e.g. `ggml-base-q5_1.bin`) but the installer takes
    // the canonical fp16 form for predictability.
    //
    // Tolerate any of these caller-side conventions so a stale config
    // value (e.g. `ggml-base-q5_1.bin` from the legacy on-demand assets
    // path) doesn't double-prefix into `ggml-ggml-base-q5_1.bin.bin`:
    //   - short token: `tiny`, `large-v3-turbo`
    //   - factory id:  `whisper-large-v3-turbo`
    //   - full ggml:   `ggml-base-q5_1.bin`
    let trimmed = size.trim();
    if trimmed.is_empty() {
        return format!("ggml-{DEFAULT_WHISPER_MODEL_SIZE}.bin");
    }
    let mut s = trimmed;
    s = s.strip_prefix("whisper-").unwrap_or(s);
    s = s.strip_prefix("ggml-").unwrap_or(s);
    s = s.strip_suffix(".bin").unwrap_or(s);
    format!("ggml-{s}.bin")
}

/// Canonical HuggingFace download URL for `ggml-<size>.bin`. Anchored on
/// `ggerganov/whisper.cpp` (the upstream-maintained bucket) so the URL
/// stays stable across whisper.cpp version bumps.
pub fn model_download_url(size: &str) -> String {
    let filename = ggml_filename(size);
    format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{filename}")
}

/// Best-effort URL for the Windows whisper-cli release archive. Returns
/// `None` on non-Windows hosts where we skip the binary fetch.
pub fn binary_download_url() -> Option<String> {
    if cfg!(windows) {
        // The Windows asset name has been stable across the recent
        // whisper.cpp releases (`whisper-bin-x64.zip`). The `latest`
        // alias on GitHub Releases follows the most recent tag.
        Some(
            "https://github.com/ggerganov/whisper.cpp/releases/latest/download/whisper-bin-x64.zip"
                .to_string(),
        )
    } else {
        None
    }
}

/// Convenience: read the current installer status snapshot.
pub fn status(config: &Config) -> VoiceInstallStatus {
    let mut snapshot = read_status(ENGINE_WHISPER);
    // If nothing has been recorded yet, derive a state from the on-disk
    // artifacts so the UI doesn't show a perpetual "missing" after a
    // successful install across a process restart.
    if matches!(snapshot.state, VoiceInstallState::Missing) {
        let configured = crate::openhuman::local_ai::model_ids::effective_stt_model_id(config);
        if installed_artifacts_ok(config, &configured) {
            snapshot.state = VoiceInstallState::Installed;
            snapshot.stage = Some(format!("{configured} present"));
        }
    }
    snapshot
}

/// Returns `true` when the workspace whisper install dir contains a
/// usable model file (model size > minimum threshold). The binary is
/// optional — the user may have whisper-cli on PATH.
fn installed_artifacts_ok(config: &Config, size: &str) -> bool {
    // Check the SPECIFIC requested size, not the default. Without this,
    // a user with `medium` installed who switches the dropdown to `small`
    // would short-circuit with "already installed" and never download
    // the new size.
    let model_path = paths::workspace_whisper_model_path(config, size);
    let model_ok = std::fs::metadata(&model_path)
        .map(|m| m.is_file() && m.len() >= MIN_MODEL_BYTES)
        .unwrap_or(false);
    log::debug!(
        "{LOG_PREFIX} install check size={size} model={} model_ok={}",
        model_path.display(),
        model_ok
    );
    model_ok
}

/// Kick off (or re-kick) a Whisper install. `force_reinstall = true`
/// removes any existing model file first; otherwise an already-installed
/// engine returns immediately with a no-op success.
pub async fn install_whisper(
    config: &Config,
    model_size: Option<String>,
    force_reinstall: bool,
) -> Result<VoiceInstallStatus, String> {
    let size = model_size
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_WHISPER_MODEL_SIZE)
        .to_string();
    log::debug!("{LOG_PREFIX} install requested size={size} force_reinstall={force_reinstall}");

    if !force_reinstall && installed_artifacts_ok(config, &size) {
        log::debug!("{LOG_PREFIX} short-circuit: artifacts already present");
        let snapshot = VoiceInstallStatus {
            engine: ENGINE_WHISPER.to_string(),
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
        engine: ENGINE_WHISPER.to_string(),
        state: VoiceInstallState::Installing,
        progress: Some(0),
        downloaded_bytes: Some(0),
        total_bytes: None,
        stage: Some(format!("starting whisper install ({size})")),
        error_detail: None,
    });

    let result = run_install(config, &size).await;
    match &result {
        Ok(()) => {
            let snapshot = VoiceInstallStatus {
                engine: ENGINE_WHISPER.to_string(),
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
                engine: ENGINE_WHISPER.to_string(),
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

async fn run_install(config: &Config, size: &str) -> Result<(), String> {
    // 1) Download the GGML model file (the must-have artifact).
    let model_path = paths::workspace_whisper_model_path(config, size);
    let model_url = model_download_url(size);
    log::debug!("{LOG_PREFIX} downloading model url={model_url}");
    update_stage(format!("downloading {}", ggml_filename(size)));
    download_to_file(
        &model_url,
        &model_path,
        None,
        MIN_MODEL_BYTES,
        LOG_PREFIX,
        |downloaded, total| {
            let progress = total
                .filter(|t| *t > 0)
                .map(|t| ((downloaded * 100) / t).min(100) as u8);
            write_status(VoiceInstallStatus {
                engine: ENGINE_WHISPER.to_string(),
                state: VoiceInstallState::Installing,
                progress,
                downloaded_bytes: Some(downloaded),
                total_bytes: total,
                stage: Some("downloading model".to_string()),
                error_detail: None,
            });
        },
    )
    .await?;
    log::debug!("{LOG_PREFIX} model staged at {}", model_path.display());

    // 2) Windows only: fetch the whisper-cli binary archive.
    if let Some(url) = binary_download_url() {
        let zip_path = paths::workspace_whisper_dir(config).join("whisper-bin-x64.zip");
        log::debug!("{LOG_PREFIX} downloading binary url={url}");
        update_stage("downloading whisper-cli binary".to_string());
        download_to_file(
            &url,
            &zip_path,
            None,
            MIN_BINARY_ZIP_BYTES,
            LOG_PREFIX,
            |downloaded, total| {
                let progress = total
                    .filter(|t| *t > 0)
                    .map(|t| ((downloaded * 100) / t).min(100) as u8);
                write_status(VoiceInstallStatus {
                    engine: ENGINE_WHISPER.to_string(),
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
        update_stage("extracting whisper-cli binary".to_string());
        extract_zip(&zip_path, &paths::workspace_whisper_dir(config))?;
        // Best-effort cleanup of the staged archive.
        let _ = std::fs::remove_file(&zip_path);
    }

    Ok(())
}

fn update_stage(stage: String) {
    let mut current = read_status(ENGINE_WHISPER);
    current.stage = Some(stage);
    write_status(current);
}

/// Extract a zip file synchronously. Whisper's Windows binary archive is
/// small (a few megabytes) so blocking is fine here — we're not on the
/// hot async path.
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
            // Skip suspicious entries (zip-slip protection).
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

/// Return the workspace-installed whisper-cli binary path if one exists.
/// Used by `paths::resolve_whisper_binary` to prefer the workspace
/// install over `WHISPER_BIN` / PATH.
pub(crate) fn find_workspace_whisper_binary(config: &Config) -> Option<PathBuf> {
    let candidates = paths::workspace_whisper_binary_candidates(config);
    for candidate in candidates {
        if candidate.is_file() {
            log::debug!(
                "{LOG_PREFIX} found workspace whisper binary at {}",
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
    fn ggml_filename_strips_whisper_prefix() {
        assert_eq!(
            ggml_filename("whisper-large-v3-turbo"),
            "ggml-large-v3-turbo.bin"
        );
        assert_eq!(ggml_filename("large-v3-turbo"), "ggml-large-v3-turbo.bin");
        assert_eq!(ggml_filename("tiny"), "ggml-tiny.bin");
        assert_eq!(ggml_filename("  base  "), "ggml-base.bin");
        // Tolerate full ggml filename (regression: stale legacy config like
        // `ggml-base-q5_1.bin` used to produce `ggml-ggml-base-q5_1.bin.bin`).
        assert_eq!(ggml_filename("ggml-base-q5_1.bin"), "ggml-base-q5_1.bin");
        assert_eq!(ggml_filename("ggml-tiny.bin"), "ggml-tiny.bin");
    }

    #[test]
    fn ggml_filename_empty_falls_back_to_default() {
        assert_eq!(
            ggml_filename(""),
            format!("ggml-{DEFAULT_WHISPER_MODEL_SIZE}.bin")
        );
        assert_eq!(
            ggml_filename("   "),
            format!("ggml-{DEFAULT_WHISPER_MODEL_SIZE}.bin")
        );
    }

    #[test]
    fn model_download_url_anchors_on_hf_bucket() {
        let url = model_download_url("tiny");
        assert!(
            url.starts_with("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/"),
            "url should anchor on the canonical HF bucket: {url}"
        );
        assert!(url.ends_with("ggml-tiny.bin"));
    }

    #[test]
    fn binary_download_url_only_for_windows() {
        if cfg!(windows) {
            let url = binary_download_url().expect("windows must offer a binary url");
            assert!(url.contains("whisper-bin-x64.zip"));
            assert!(url.contains("github.com/ggerganov/whisper.cpp"));
        } else {
            assert!(
                binary_download_url().is_none(),
                "non-Windows hosts should not advertise a binary download URL"
            );
        }
    }

    /// Serialise tests that write into the shared `~/.openhuman/bin/whisper/`
    /// directory. `shared_root_dir` ignores `config.workspace_dir` and goes
    /// straight to the user home dir, so two tests can collide if they run
    /// in parallel. Reuses the module-wide `local_ai_test_guard` so paths
    /// + install_piper tests are serialised through the same lock.
    fn shared_install_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::openhuman::local_ai::local_ai_test_guard()
    }

    /// Wipe the shared-root install dir for whisper so the absence
    /// assertions below are deterministic across parallel test runs.
    fn wipe_shared_install_dir(config: &Config) {
        let dir = paths::workspace_whisper_dir(config);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn status_reports_missing_for_fresh_workspace() {
        let _g = shared_install_lock();
        reset_status(ENGINE_WHISPER);
        let (_tmp, config) = temp_config();
        wipe_shared_install_dir(&config);
        let snapshot = status(&config);
        assert_eq!(snapshot.state, VoiceInstallState::Missing);
    }

    #[test]
    fn status_promotes_to_installed_when_model_present() {
        let _g = shared_install_lock();
        reset_status(ENGINE_WHISPER);
        let (_tmp, mut config) = temp_config();
        // The status helper derives installed state from effective_stt_model_id,
        // so config must agree with the file we create. Pin it to the default
        // model size so the on-disk lookup matches.
        config.local_ai.stt_model_id = DEFAULT_WHISPER_MODEL_SIZE.to_string();
        wipe_shared_install_dir(&config);
        let path = paths::workspace_whisper_model_path(&config, DEFAULT_WHISPER_MODEL_SIZE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Write a file just over the size floor so the validator accepts it.
        let buf = vec![0u8; (MIN_MODEL_BYTES + 1024) as usize];
        std::fs::write(&path, &buf).unwrap();
        let snapshot = status(&config);
        assert_eq!(snapshot.state, VoiceInstallState::Installed);
        wipe_shared_install_dir(&config);
    }

    // We deliberately hold the sync mutex across the install await — the
    // install path doesn't acquire any other locks so there is no risk of
    // deadlock, and the guard's only job is to serialise filesystem
    // writes against parallel tests. Same pattern used elsewhere in
    // local_ai test modules.
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn install_short_circuits_when_already_installed() {
        let _g = shared_install_lock();
        reset_status(ENGINE_WHISPER);
        let (_tmp, config) = temp_config();
        wipe_shared_install_dir(&config);
        let path = paths::workspace_whisper_model_path(&config, DEFAULT_WHISPER_MODEL_SIZE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let buf = vec![0u8; (MIN_MODEL_BYTES + 1024) as usize];
        std::fs::write(&path, &buf).unwrap();

        let result = install_whisper(&config, None, false).await;
        assert!(result.is_ok(), "short-circuit must succeed: {result:?}");
        let snap = result.unwrap();
        assert_eq!(snap.state, VoiceInstallState::Installed);
        assert_eq!(snap.stage.as_deref(), Some("already installed"));
        wipe_shared_install_dir(&config);
    }

    #[test]
    fn find_workspace_whisper_binary_returns_none_without_install() {
        let _g = shared_install_lock();
        let (_tmp, config) = temp_config();
        wipe_shared_install_dir(&config);
        assert!(find_workspace_whisper_binary(&config).is_none());
    }

    #[test]
    fn find_workspace_whisper_binary_returns_path_when_present() {
        let _g = shared_install_lock();
        let (_tmp, config) = temp_config();
        wipe_shared_install_dir(&config);
        let candidates = paths::workspace_whisper_binary_candidates(&config);
        let target = candidates.first().expect("at least one candidate").clone();
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"stub").unwrap();
        let found = find_workspace_whisper_binary(&config).expect("should find binary");
        assert_eq!(found, target);
        wipe_shared_install_dir(&config);
    }
}
