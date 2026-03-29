//! Workspace paths for Ollama, Whisper, Piper, and downloaded assets.

use std::path::PathBuf;

use crate::openhuman::config::Config;

use super::model_ids;

pub(crate) fn config_root_dir(config: &Config) -> PathBuf {
    config
        .config_path
        .parent()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| config.workspace_dir.clone())
}

pub(crate) fn workspace_ollama_dir(config: &Config) -> PathBuf {
    config_root_dir(config).join("bin").join("ollama")
}

pub(crate) fn workspace_ollama_binary(config: &Config) -> PathBuf {
    let name = if cfg!(windows) {
        "ollama.exe"
    } else {
        "ollama"
    };
    workspace_ollama_dir(config).join(name)
}

pub(crate) fn workspace_local_models_dir(config: &Config) -> PathBuf {
    config_root_dir(config).join("models").join("local-ai")
}

pub(crate) fn resolve_whisper_binary() -> Option<PathBuf> {
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

pub(crate) fn resolve_piper_binary() -> Option<PathBuf> {
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

pub(crate) fn resolve_stt_model_path(config: &Config) -> Result<String, String> {
    let id = model_ids::effective_stt_model_id(config);
    let path = PathBuf::from(&id);
    if path.is_file() {
        return Ok(path.display().to_string());
    }
    let candidate = workspace_local_models_dir(config).join("stt").join(&id);
    if candidate.is_file() {
        Ok(candidate.display().to_string())
    } else {
        Err(format!(
            "STT model not found. Expected '{}' or '{}'",
            path.display(),
            candidate.display()
        ))
    }
}

pub(crate) fn resolve_tts_voice_path(config: &Config) -> Result<String, String> {
    let voice_id = model_ids::effective_tts_voice_id(config);
    let path = PathBuf::from(&voice_id);
    if path.is_file() {
        return Ok(path.display().to_string());
    }
    let filename = if voice_id.ends_with(".onnx") {
        voice_id
    } else {
        format!("{voice_id}.onnx")
    };
    let candidate = workspace_local_models_dir(config)
        .join("tts")
        .join(filename);
    if candidate.is_file() {
        Ok(candidate.display().to_string())
    } else {
        Err(format!(
            "TTS voice model not found. Expected '{}' or '{}'",
            path.display(),
            candidate.display()
        ))
    }
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
