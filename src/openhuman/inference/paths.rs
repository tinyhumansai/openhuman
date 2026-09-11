//! Workspace paths for Ollama, Piper, and downloaded assets.

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

/// Standard Unix locations a CLI binary may live in that are **not**
/// guaranteed to be on the `PATH` a GUI app inherits. A macOS app launched
/// from Finder/Dock gets the minimal launchd `PATH`
/// (`/usr/bin:/bin:/usr/sbin:/sbin`), so Homebrew dirs (`/opt/homebrew/bin`
/// on Apple Silicon, `/usr/local/bin` on Intel) are invisible even when the
/// user installed the binary there and it runs fine from a terminal — the
/// exact symptom in issue #3425. Probe these explicitly as a last resort.
///
/// Windows resolution relies entirely on the `PATH` scan, so this is empty
/// there (the in-app installer drops its binaries into the workspace anyway).
fn standard_unix_bin_dirs() -> Vec<PathBuf> {
    if cfg!(windows) {
        return Vec::new();
    }
    [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/bin",
        "/usr/sbin",
        "/sbin",
    ]
    .iter()
    .map(PathBuf::from)
    .collect()
}

/// Return the first of `dirs` that holds `bin_name` as a regular file.
/// Shared by the `PATH` scan and the standard-dir fallback so both agree on
/// what "found" means.
fn resolve_binary_in_dirs(bin_name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join(bin_name))
        .find(|candidate| candidate.is_file())
}

pub(crate) fn resolve_piper_binary() -> Option<PathBuf> {
    // Precedence: workspace install > env override > PATH lookup. The
    // workspace install path is the canonical drop-zone populated by
    // `install_piper::install_piper`; checking it first means a user who just
    // clicked Install in the VoicePanel doesn't also have to export PIPER_BIN.
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
    if let Some(from_path) = std::env::var_os("PATH").and_then(|path_var| {
        let dirs: Vec<PathBuf> = std::env::split_paths(&path_var).collect();
        resolve_binary_in_dirs(bin_name, &dirs)
    }) {
        return Some(from_path);
    }

    // Last resort: GUI-app PATH omits Homebrew dirs (see
    // `standard_unix_bin_dirs`). Probe them so a `brew install piper` binary
    // is found even when launched from Finder.
    if let Some(from_std) = resolve_binary_in_dirs(bin_name, &standard_unix_bin_dirs()) {
        log::debug!(
            "[voice-install:piper] resolved binary from standard dir {}",
            from_std.display()
        );
        return Some(from_std);
    }
    None
}

/// Config-aware piper resolution: workspace install first, env second,
/// PATH third.
pub(crate) fn resolve_piper_binary_with_config(config: &Config) -> Option<PathBuf> {
    if let Some(workspace) =
        crate::openhuman::inference::local::install_piper::find_workspace_piper_binary(config)
    {
        return Some(workspace);
    }
    resolve_piper_binary()
}

// ---------------------------------------------------------------------------
// Workspace install paths — used by install_piper and the local-AI asset
// downloader.
// ---------------------------------------------------------------------------

/// Workspace dir for downloaded STT model files. Lives next to the Ollama dir
/// so users with a single shared root see all local-AI artifacts together. The
/// `whisper` leaf is retained verbatim so an existing install keeps resolving
/// after the bundled whisper.cpp engine was removed.
pub(crate) fn workspace_whisper_dir(config: &Config) -> PathBuf {
    shared_root_dir(config).join("bin").join("whisper")
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
#[path = "paths_tests.rs"]
mod tests;
