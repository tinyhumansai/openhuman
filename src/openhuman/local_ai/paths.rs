//! Workspace paths for Ollama, Whisper, Piper, and downloaded assets.

use std::path::PathBuf;

use crate::openhuman::config::Config;

use super::model_ids;

/// Returns the per-user config directory (parent of config.toml).
pub(crate) fn config_root_dir(config: &Config) -> PathBuf {
    config
        .config_path
        .parent()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| config.workspace_dir.clone())
}

/// Returns the root directory under which local-AI artifacts (binaries,
/// model files) are written and resolved.
///
/// Default callers see the shared `~/.openhuman/` root, which avoids
/// duplicating multi-GB model files across users on a single machine.
///
/// When `OPENHUMAN_WORKSPACE` is **explicitly** set (test/dev parallel
/// sessions, multi-workspace deployments, isolated CI runs), the
/// shared-root contract no longer applies — those callers want full
/// isolation, including their own copy of any installed binaries. Honor
/// the override by returning the workspace dir directly.
fn shared_root_dir(config: &Config) -> PathBuf {
    if std::env::var_os("OPENHUMAN_WORKSPACE").is_some() {
        return config_root_dir(config);
    }
    crate::openhuman::config::default_root_openhuman_dir()
        .unwrap_or_else(|_| config_root_dir(config))
}

pub(crate) fn workspace_ollama_dir(config: &Config) -> PathBuf {
    shared_root_dir(config).join("bin").join("ollama")
}

pub(crate) fn workspace_ollama_binary(config: &Config) -> PathBuf {
    if cfg!(target_os = "linux") {
        return workspace_ollama_dir(config).join("bin").join("ollama");
    }

    let name = if cfg!(windows) {
        "ollama.exe"
    } else {
        "ollama"
    };
    workspace_ollama_dir(config).join(name)
}

pub(crate) fn workspace_ollama_binary_candidates(config: &Config) -> Vec<PathBuf> {
    let dir = workspace_ollama_dir(config);
    let binary_name = if cfg!(windows) {
        "ollama.exe"
    } else {
        "ollama"
    };

    let mut candidates = Vec::new();
    if cfg!(target_os = "linux") {
        candidates.push(dir.join("bin").join(binary_name));
    }
    candidates.push(dir.join(binary_name));
    candidates.push(
        dir.join("Ollama.app")
            .join("Contents")
            .join("Resources")
            .join(binary_name),
    );
    candidates
}

pub(crate) fn find_workspace_ollama_binary(config: &Config) -> Option<PathBuf> {
    workspace_ollama_binary_candidates(config)
        .into_iter()
        .find(|candidate| candidate.is_file())
}

pub(crate) fn workspace_local_models_dir(config: &Config) -> PathBuf {
    shared_root_dir(config).join("models").join("local-ai")
}

/// Spawn marker file recording the PID of any `ollama serve` openhuman
/// itself spawned. Read on next launch to recognise our own orphan when
/// openhuman crashed before its graceful-shutdown hook ran. Lives under
/// the shared root so it survives per-user config rewrites and sits next
/// to the workspace install dir.
pub(crate) fn ollama_spawn_marker_path(config: &Config) -> PathBuf {
    shared_root_dir(config)
        .join("local-ai")
        .join("ollama.spawn")
}

pub(crate) fn resolve_whisper_binary() -> Option<PathBuf> {
    // Precedence: workspace install > env override > PATH lookup. The
    // workspace install path is the canonical drop-zone for the binary
    // populated by `install_whisper::install_whisper`; checking it first
    // means a user who just clicked Install in the VoicePanel doesn't
    // have to also export WHISPER_BIN. Falls back to the env+PATH form
    // for advanced users who pin a custom binary.
    if let Ok(shared) = crate::openhuman::config::default_root_openhuman_dir() {
        let root = shared.join("bin").join("whisper");
        let bin_name = if cfg!(windows) {
            "whisper-cli.exe"
        } else {
            "whisper-cli"
        };
        for candidate in [
            root.join(bin_name),
            root.join("whisper-bin-x64").join(bin_name),
            root.join("bin").join(bin_name),
        ] {
            if candidate.is_file() {
                log::debug!(
                    "[voice-install:whisper] resolved workspace binary {}",
                    candidate.display()
                );
                return Some(candidate);
            }
        }
    }

    if let Some(from_env) = std::env::var("WHISPER_BIN")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        let path = PathBuf::from(from_env);
        if path.is_file() {
            return Some(path);
        }
    }

    let bin_name = if cfg!(windows) {
        "whisper-cli.exe"
    } else {
        "whisper-cli"
    };
    std::env::var_os("PATH").and_then(|path_var| {
        std::env::split_paths(&path_var)
            .map(|entry| entry.join(bin_name))
            .find(|candidate| candidate.is_file())
    })
}

/// Config-aware whisper resolution. Preference order:
///   1. Workspace-installed binary (placed by `install_whisper`)
///   2. `WHISPER_BIN` env override
///   3. `whisper-cli` on PATH
///
/// Falling back to the env-only resolver lets callers that don't have a
/// `Config` reference (e.g. the bare-process voice STT subprocess code)
/// stay compiling without rewiring.
pub(crate) fn resolve_whisper_binary_with_config(config: &Config) -> Option<PathBuf> {
    if let Some(workspace) = super::install_whisper::find_workspace_whisper_binary(config) {
        return Some(workspace);
    }
    resolve_whisper_binary()
}

pub(crate) fn resolve_piper_binary() -> Option<PathBuf> {
    // Precedence: workspace install > env override > PATH lookup. See
    // the `resolve_whisper_binary` comment above for the rationale.
    if let Ok(shared) = crate::openhuman::config::default_root_openhuman_dir() {
        let root = shared.join("bin").join("piper");
        let bin_name = if cfg!(windows) { "piper.exe" } else { "piper" };
        for candidate in [
            root.join(bin_name),
            root.join("piper").join(bin_name),
            root.join("bin").join(bin_name),
        ] {
            if candidate.is_file() {
                log::debug!(
                    "[voice-install:piper] resolved workspace binary {}",
                    candidate.display()
                );
                return Some(candidate);
            }
        }
    }

    if let Some(from_env) = std::env::var("PIPER_BIN")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        let path = PathBuf::from(from_env);
        if path.is_file() {
            return Some(path);
        }
    }

    let bin_name = if cfg!(windows) { "piper.exe" } else { "piper" };
    std::env::var_os("PATH").and_then(|path_var| {
        std::env::split_paths(&path_var)
            .map(|entry| entry.join(bin_name))
            .find(|candidate| candidate.is_file())
    })
}

/// Config-aware piper resolution. Same precedence shape as
/// `resolve_whisper_binary_with_config` — workspace install first, env
/// second, PATH third.
pub(crate) fn resolve_piper_binary_with_config(config: &Config) -> Option<PathBuf> {
    if let Some(workspace) = super::install_piper::find_workspace_piper_binary(config) {
        return Some(workspace);
    }
    resolve_piper_binary()
}

// ---------------------------------------------------------------------------
// Workspace install paths — used by install_whisper / install_piper.
// ---------------------------------------------------------------------------

/// Workspace dir for Whisper artifacts. Lives next to the Ollama dir so
/// users with a single shared root see all local-AI binaries together.
pub(crate) fn workspace_whisper_dir(config: &Config) -> PathBuf {
    shared_root_dir(config).join("bin").join("whisper")
}

/// On-disk path for the GGML model file. `size` is the short
/// designator (`tiny`, `base`, `small`, `medium`, `large-v3-turbo`).
///
/// Tolerates any of these caller-side conventions so a stale config
/// value (e.g. legacy `ggml-base-q5_1.bin`) doesn't produce the broken
/// `ggml-ggml-base-q5_1.bin.bin` filename and break the
/// "is whisper installed?" resolver:
///   - short token: `tiny`, `large-v3-turbo`
///   - factory id:  `whisper-large-v3-turbo`
///   - full ggml:   `ggml-base-q5_1.bin`
pub(crate) fn workspace_whisper_model_path(config: &Config, size: &str) -> PathBuf {
    let trimmed = size.trim();
    if trimmed.is_empty() {
        return workspace_whisper_dir(config).join("ggml-medium.bin");
    }
    let mut s = trimmed;
    s = s.strip_prefix("whisper-").unwrap_or(s);
    s = s.strip_prefix("ggml-").unwrap_or(s);
    s = s.strip_suffix(".bin").unwrap_or(s);
    workspace_whisper_dir(config).join(format!("ggml-{s}.bin"))
}

/// All candidate paths where the workspace-installed whisper-cli binary
/// might land after extraction. The Windows archive nests the binary
/// inside a `whisper-bin-x64/` directory; check both the flat and
/// nested layouts so future archive shape changes don't silently break
/// resolution.
pub(crate) fn workspace_whisper_binary_candidates(config: &Config) -> Vec<PathBuf> {
    let root = workspace_whisper_dir(config);
    let bin_name = if cfg!(windows) {
        "whisper-cli.exe"
    } else {
        "whisper-cli"
    };
    // Layouts observed in upstream releases:
    //   - Windows zip extracts to `Release/` (cmake build artifact dir)
    //   - Older archives flattened to root or used `whisper-bin-x64/`
    //   - Some package managers drop the binary in `bin/`
    // Probe every known layout so future archive shape changes don't
    // silently break resolution.
    vec![
        root.join(bin_name),
        root.join("Release").join(bin_name),
        root.join("whisper-bin-x64").join(bin_name),
        root.join("whisper-bin-x64").join("Release").join(bin_name),
        root.join("bin").join(bin_name),
    ]
}

/// Workspace dir for Piper artifacts.
pub(crate) fn workspace_piper_dir(config: &Config) -> PathBuf {
    shared_root_dir(config).join("bin").join("piper")
}

/// On-disk paths for a Piper voice — returns the `.onnx` and
/// `.onnx.json` sidecar in that order. Returns `None` if the voice id
/// is empty (no fallback — the caller must validate up front).
pub(crate) fn workspace_piper_voice_paths(
    config: &Config,
    voice_id: &str,
) -> Option<(PathBuf, PathBuf)> {
    let trimmed = voice_id.trim();
    if trimmed.is_empty() {
        return None;
    }
    let base = workspace_piper_dir(config).join("voices").join(trimmed);
    Some((
        base.with_extension("onnx"),
        base.with_extension("onnx.json"),
    ))
}

/// All candidate paths where the workspace-installed Piper binary might
/// land. Windows zips drop `piper.exe` in a `piper/` subdir; tar.gz
/// archives on Linux/macOS sometimes flatten to the install root.
pub(crate) fn workspace_piper_binary_candidates(config: &Config) -> Vec<PathBuf> {
    let root = workspace_piper_dir(config);
    let bin_name = if cfg!(windows) { "piper.exe" } else { "piper" };
    vec![
        root.join(bin_name),
        root.join("piper").join(bin_name),
        root.join("bin").join(bin_name),
    ]
}

pub(crate) fn resolve_stt_model_path(config: &Config) -> Result<String, String> {
    let id = model_ids::effective_stt_model_id(config);
    resolve_stt_model_path_by_id(&id, config)
}

/// Resolve the on-disk GGML model path for an explicit `model_id`.
///
/// Used when the caller has already computed the effective model id (e.g.
/// from a per-request override) and needs the path without re-reading the
/// config default. Probes the same candidate set as `resolve_stt_model_path`.
pub(crate) fn resolve_stt_model_path_by_id(id: &str, config: &Config) -> Result<String, String> {
    let path = PathBuf::from(id);
    if path.is_file() {
        return Ok(path.display().to_string());
    }
    // The voice installer places the GGML model file under
    // `workspace_whisper_dir(config)/ggml-<size>.bin`, but the legacy
    // local-AI flow stages STT models under `workspace_local_models_dir`.
    // Probe both so a user who installed via the new Install button
    // doesn't need to redo anything.
    let legacy = workspace_local_models_dir(config).join("stt").join(id);
    if legacy.is_file() {
        return Ok(legacy.display().to_string());
    }
    let installer = workspace_whisper_dir(config).join(id);
    if installer.is_file() {
        return Ok(installer.display().to_string());
    }
    // Also probe the ggml-prefixed form for short ids like `tiny`.
    let bare = id.trim().strip_prefix("whisper-").unwrap_or(id.trim());
    let normalized = if bare.starts_with("ggml-") {
        bare.to_string()
    } else {
        format!("ggml-{bare}.bin")
    };
    let normalized_path = workspace_whisper_dir(config).join(&normalized);
    if normalized_path.is_file() {
        return Ok(normalized_path.display().to_string());
    }
    Err(format!(
        "STT model not found. Expected one of '{}', '{}', '{}', '{}'",
        path.display(),
        legacy.display(),
        installer.display(),
        normalized_path.display()
    ))
}

pub(crate) fn resolve_tts_voice_path(config: &Config) -> Result<String, String> {
    let voice_id = model_ids::effective_tts_voice_id(config);
    let path = PathBuf::from(&voice_id);
    if path.is_file() {
        return Ok(path.display().to_string());
    }
    let filename = if voice_id.ends_with(".onnx") {
        voice_id.clone()
    } else {
        format!("{voice_id}.onnx")
    };
    // Installer drop-zone — `install_piper` writes
    // `bin/piper/voices/<id>.onnx`. Probed FIRST because legacy paths
    // may contain stale stubs from earlier workspaces (a 4-byte legacy
    // stub used to win over a 63 MB installer copy and crash Piper with
    // STATUS_STACK_BUFFER_OVERRUN).
    let installer_onnx_path =
        workspace_piper_voice_paths(config, voice_id.trim_end_matches(".onnx"))
            .map(|(onnx, _)| onnx);
    if let Some(p) = &installer_onnx_path {
        if p.is_file() {
            return Ok(p.display().to_string());
        }
    }
    // Legacy path used by the original voice pipeline. Still checked so
    // pre-installer setups keep working.
    let legacy = workspace_local_models_dir(config)
        .join("tts")
        .join(&filename);
    if legacy.is_file() {
        return Ok(legacy.display().to_string());
    }
    let installer_display = installer_onnx_path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(no installer path resolvable)".to_string());
    Err(format!(
        "TTS voice model not found. Expected '{}' (installer) or '{}' (legacy)",
        installer_display,
        legacy.display()
    ))
}

pub(crate) fn stt_model_target_path(config: &Config) -> PathBuf {
    let id = model_ids::effective_stt_model_id(config);
    let path = PathBuf::from(&id);
    if path.is_absolute() {
        path
    } else {
        workspace_local_models_dir(config).join("stt").join(id)
    }
}

pub(crate) fn tts_model_target_path(config: &Config) -> PathBuf {
    let voice_id = model_ids::effective_tts_voice_id(config);
    let path = PathBuf::from(&voice_id);
    if path.is_absolute() {
        return path;
    }
    let filename = if voice_id.ends_with(".onnx") {
        voice_id
    } else {
        format!("{voice_id}.onnx")
    };
    workspace_local_models_dir(config)
        .join("tts")
        .join(filename)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config() -> (tempfile::TempDir, Config) {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = Config::default();
        config.workspace_dir = dir.path().join("workspace");
        config.config_path = dir.path().join("config.toml");
        (dir, config)
    }

    #[test]
    fn resolve_stt_model_path_prefers_workspace_relative_artifact() {
        let (_tmp, mut config) = temp_config();
        config.local_ai.stt_model_id = "tiny.bin".to_string();
        let model_path = workspace_local_models_dir(&config)
            .join("stt")
            .join("tiny.bin");
        std::fs::create_dir_all(model_path.parent().expect("parent")).expect("mkdirs");
        std::fs::write(&model_path, b"stub").expect("write");

        let resolved = resolve_stt_model_path(&config).expect("resolve stt");
        assert_eq!(resolved, model_path.display().to_string());
    }

    #[test]
    fn resolve_tts_voice_path_appends_onnx_for_voice_ids() {
        // The installer drop-zone (`bin/piper/voices/<id>.onnx`) is probed
        // FIRST by `resolve_tts_voice_path`, and lives under the shared
        // root (`~/.openhuman/`) — not the temp config. If a sibling
        // install_piper test runs in parallel with the default voice id
        // and leaves a stub there, this test sees that file and the
        // assertion fails. Serialise via the shared install guard and
        // wipe the installer path so the legacy `models/local-ai/tts/`
        // candidate is the only match.
        let _g = shared_install_lock();
        let (_tmp, mut config) = temp_config();
        config.local_ai.tts_voice_id = "en_US-lessac-medium".to_string();
        let installer_onnx = workspace_piper_voice_paths(&config, "en_US-lessac-medium")
            .map(|(onnx, _)| onnx)
            .expect("installer onnx path");
        let _ = std::fs::remove_file(&installer_onnx);
        let model_path = workspace_local_models_dir(&config)
            .join("tts")
            .join("en_US-lessac-medium.onnx");
        std::fs::create_dir_all(model_path.parent().expect("parent")).expect("mkdirs");
        std::fs::write(&model_path, b"stub").expect("write");

        let resolved = resolve_tts_voice_path(&config).expect("resolve tts");
        assert_eq!(resolved, model_path.display().to_string());
    }

    #[test]
    fn target_paths_preserve_absolute_overrides() {
        let (_tmp, mut config) = temp_config();
        let stt = if cfg!(windows) {
            "C:\\tmp\\stt-model.bin"
        } else {
            "/tmp/stt-model.bin"
        };
        let tts = if cfg!(windows) {
            "C:\\tmp\\voice.onnx"
        } else {
            "/tmp/voice.onnx"
        };
        config.local_ai.stt_model_id = stt.to_string();
        config.local_ai.tts_voice_id = tts.to_string();

        assert_eq!(stt_model_target_path(&config), PathBuf::from(stt));
        assert_eq!(tts_model_target_path(&config), PathBuf::from(tts));
    }

    #[test]
    fn workspace_ollama_binary_matches_platform_layout() {
        let (_tmp, config) = temp_config();
        let root = workspace_ollama_dir(&config);

        if cfg!(target_os = "linux") {
            assert_eq!(
                workspace_ollama_binary(&config),
                root.join("bin").join("ollama")
            );
        } else if cfg!(windows) {
            assert_eq!(workspace_ollama_binary(&config), root.join("ollama.exe"));
        } else {
            assert_eq!(workspace_ollama_binary(&config), root.join("ollama"));
        }
    }

    #[test]
    fn find_workspace_ollama_binary_supports_legacy_flat_layout() {
        let (_tmp, config) = temp_config();
        let dir = workspace_ollama_dir(&config);
        std::fs::create_dir_all(&dir).expect("create workspace ollama dir");

        let legacy = dir.join(if cfg!(windows) {
            "ollama.exe"
        } else {
            "ollama"
        });
        std::fs::write(&legacy, b"stub").expect("write legacy binary");

        let found = find_workspace_ollama_binary(&config).expect("find workspace binary");
        assert_eq!(found, legacy);
    }

    #[test]
    fn workspace_whisper_model_path_uses_ggml_naming() {
        let (_tmp, config) = temp_config();
        let path = workspace_whisper_model_path(&config, "large-v3-turbo");
        assert!(
            path.to_string_lossy().ends_with("ggml-large-v3-turbo.bin"),
            "expected ggml-<size>.bin suffix: {}",
            path.display()
        );
        // Stripping the `whisper-` prefix keeps the filename uniform with
        // bare-size callers.
        let alt = workspace_whisper_model_path(&config, "whisper-tiny");
        assert!(alt.to_string_lossy().ends_with("ggml-tiny.bin"));
        // Regression: stale legacy config (`ggml-base-q5_1.bin`) used to
        // produce the broken path `ggml-ggml-base-q5_1.bin.bin`.
        let legacy = workspace_whisper_model_path(&config, "ggml-base-q5_1.bin");
        assert!(
            legacy.to_string_lossy().ends_with("ggml-base-q5_1.bin"),
            "stale legacy id must collapse to canonical ggml-<size>.bin: {}",
            legacy.display()
        );
        let legacy_short = workspace_whisper_model_path(&config, "ggml-tiny.bin");
        assert!(legacy_short.to_string_lossy().ends_with("ggml-tiny.bin"));
        // Empty size falls back to the default model size (medium).
        let default = workspace_whisper_model_path(&config, "");
        assert!(
            default.to_string_lossy().ends_with("ggml-medium.bin"),
            "empty size should fall back to ggml-medium.bin: {}",
            default.display()
        );
    }

    #[test]
    fn workspace_whisper_binary_candidates_cover_known_archive_layouts() {
        let (_tmp, config) = temp_config();
        let candidates = workspace_whisper_binary_candidates(&config);
        let suffix = if cfg!(windows) {
            "whisper-cli.exe"
        } else {
            "whisper-cli"
        };
        assert!(
            candidates.iter().any(|p| p.ends_with(suffix)),
            "flat-layout candidate must contain whisper-cli"
        );
        assert!(
            candidates
                .iter()
                .any(|p| p.to_string_lossy().contains("whisper-bin-x64")),
            "legacy Windows-zip nested layout must be a candidate"
        );
        // Regression: upstream Windows zip extracts to `Release/`. Without
        // this candidate, the resolver reports "binary not found" even
        // though the install succeeded.
        assert!(
            candidates
                .iter()
                .any(|p| p.to_string_lossy().contains("Release")),
            "Release/ cmake-build layout must be a candidate"
        );
    }

    #[test]
    fn workspace_piper_voice_paths_returns_onnx_pair() {
        let (_tmp, config) = temp_config();
        let (onnx, json) =
            workspace_piper_voice_paths(&config, "en_US-lessac-medium").expect("voice paths");
        assert!(onnx.to_string_lossy().ends_with("en_US-lessac-medium.onnx"));
        assert!(json
            .to_string_lossy()
            .ends_with("en_US-lessac-medium.onnx.json"));
        // Empty voice id is rejected so the caller can fail fast.
        assert!(workspace_piper_voice_paths(&config, "").is_none());
        assert!(workspace_piper_voice_paths(&config, "   ").is_none());
    }

    #[test]
    fn workspace_piper_binary_candidates_include_flat_layout() {
        let (_tmp, config) = temp_config();
        let candidates = workspace_piper_binary_candidates(&config);
        let suffix = if cfg!(windows) { "piper.exe" } else { "piper" };
        assert!(
            candidates.iter().any(|p| p.ends_with(suffix)),
            "flat-layout piper binary must be a candidate"
        );
    }

    /// Serialise with sibling install_whisper / install_piper tests that
    /// write into the same shared `~/.openhuman/bin/...` directory. Uses
    /// the existing module-wide guard so all readers/writers go through
    /// one critical section.
    fn shared_install_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::openhuman::local_ai::local_ai_test_guard()
    }

    #[test]
    fn resolve_whisper_binary_with_config_prefers_workspace_install() {
        // The workspace candidate takes precedence over PATH lookup. We
        // can't trivially clear PATH on every host, but writing a stub
        // into the workspace dir is enough to verify the function
        // returns the workspace path first.
        let _g = shared_install_lock();
        let (_tmp, config) = temp_config();
        let target = workspace_whisper_binary_candidates(&config)
            .into_iter()
            .next()
            .expect("at least one candidate");
        // Wipe + recreate so a leftover stub from a parallel test cannot
        // race the mkdir/write pair below.
        let _ = std::fs::remove_dir_all(workspace_whisper_dir(&config));
        std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
        std::fs::write(&target, b"stub").expect("write stub");
        let resolved = resolve_whisper_binary_with_config(&config).expect("workspace resolve");
        assert_eq!(resolved, target);
        let _ = std::fs::remove_dir_all(workspace_whisper_dir(&config));
    }

    #[test]
    fn resolve_piper_binary_with_config_prefers_workspace_install() {
        let _g = shared_install_lock();
        let (_tmp, config) = temp_config();
        let target = workspace_piper_binary_candidates(&config)
            .into_iter()
            .next()
            .expect("at least one candidate");
        let _ = std::fs::remove_dir_all(workspace_piper_dir(&config));
        std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
        std::fs::write(&target, b"stub").expect("write stub");
        let resolved = resolve_piper_binary_with_config(&config).expect("workspace resolve");
        assert_eq!(resolved, target);
        let _ = std::fs::remove_dir_all(workspace_piper_dir(&config));
    }
}
