//! Voice domain business logic — STT (whisper.cpp) and TTS (piper).
//!
//! Each public function follows the `RpcOutcome<T>` pattern used by other
//! domain modules (billing, health, etc.).

use chrono::Utc;
use log::{debug, warn};
use std::time::Instant;

use crate::openhuman::config::Config;
use crate::openhuman::local_ai;
use crate::openhuman::local_ai::model_ids;
use crate::openhuman::local_ai::paths::{
    resolve_piper_binary, resolve_stt_model_path, resolve_tts_voice_path, resolve_whisper_binary,
};
use crate::openhuman::local_ai::whisper_engine;
use crate::rpc::RpcOutcome;

use super::postprocess;
use super::types::{VoiceSpeechResult, VoiceStatus, VoiceTtsResult};

const LOG_PREFIX: &str = "[voice]";

/// Check availability of STT/TTS binaries and models without executing them.
pub async fn voice_status(config: &Config) -> Result<RpcOutcome<VoiceStatus>, String> {
    debug!("{LOG_PREFIX} checking voice status");

    let whisper_bin = resolve_whisper_binary();
    let piper_bin = resolve_piper_binary();
    let stt_model = resolve_stt_model_path(config).ok();
    let tts_voice = resolve_tts_voice_path(config).ok();

    let service = local_ai::global(config);
    let whisper_in_process = whisper_engine::is_loaded(&service.whisper);

    // STT is available when ANY transcription backend can work:
    // 1. The in-process whisper engine is already loaded, OR
    // 2. In-process whisper is enabled in config and the model file exists
    //    (the engine will load the model on first use), OR
    // 3. The whisper-cli binary is installed and the model file exists.
    let stt_available = whisper_in_process
        || (config.local_ai.whisper_in_process && stt_model.is_some())
        || (whisper_bin.is_some() && stt_model.is_some());
    let tts_available = piper_bin.is_some() && tts_voice.is_some();

    debug!(
        "{LOG_PREFIX} stt_available={stt_available} tts_available={tts_available} \
         whisper_in_process={whisper_in_process} \
         whisper_bin={} piper_bin={} stt_model={} tts_voice={}",
        safe_basename_path(&whisper_bin),
        safe_basename_path(&piper_bin),
        safe_basename_str(&stt_model),
        safe_basename_str(&tts_voice),
    );

    let status = VoiceStatus {
        stt_available,
        tts_available,
        stt_model_id: model_ids::effective_stt_model_id(config),
        tts_voice_id: model_ids::effective_tts_voice_id(config),
        whisper_binary: whisper_bin.map(|p| p.display().to_string()),
        piper_binary: piper_bin.map(|p| p.display().to_string()),
        stt_model_path: stt_model,
        tts_voice_path: tts_voice,
        whisper_in_process,
        llm_cleanup_enabled: config.local_ai.voice_llm_cleanup_enabled,
    };

    Ok(RpcOutcome::single_log(status, "voice status checked"))
}

/// Transcribe audio from a file path using whisper.cpp.
///
/// If `context` is provided, the raw transcription is post-processed through
/// a local LLM to fix grammar and disambiguate words using conversation history.
pub async fn voice_transcribe(
    config: &Config,
    audio_path: &str,
    context: Option<&str>,
    skip_cleanup: bool,
) -> Result<RpcOutcome<VoiceSpeechResult>, String> {
    let started = Instant::now();
    debug!("{LOG_PREFIX} transcribing audio_path={audio_path}");

    let service = local_ai::global(config);
    let transcribe_started = Instant::now();
    // Pass context as initial_prompt to bias whisper toward known vocabulary.
    let output = service
        .transcribe_with_prompt(config, audio_path.trim(), context)
        .await
        .map_err(|e| e.to_string())?;
    let transcribe_elapsed = transcribe_started.elapsed();

    let raw_text = output.text.clone();
    debug!(
        "{LOG_PREFIX} transcription completed, text length={}, stt_elapsed_ms={}",
        raw_text.len(),
        transcribe_elapsed.as_millis()
    );

    let cleanup_started = Instant::now();
    let text = if skip_cleanup {
        raw_text.clone()
    } else {
        postprocess::cleanup_transcription(config, &raw_text, context).await
    };
    let cleanup_elapsed = cleanup_started.elapsed();
    debug!(
        "{LOG_PREFIX} voice_transcribe complete (cleanup_elapsed_ms={}, total_elapsed_ms={})",
        cleanup_elapsed.as_millis(),
        started.elapsed().as_millis()
    );

    Ok(RpcOutcome::single_log(
        VoiceSpeechResult {
            text,
            raw_text,
            model_id: output.model_id,
        },
        "voice transcription completed",
    ))
}

/// Transcribe audio from raw bytes. Writes to a temp file, transcribes, cleans up.
///
/// If `context` is provided, the raw transcription is post-processed through
/// a local LLM.
pub async fn voice_transcribe_bytes(
    config: &Config,
    audio_bytes: &[u8],
    extension: Option<String>,
    context: Option<&str>,
    skip_cleanup: bool,
) -> Result<RpcOutcome<VoiceSpeechResult>, String> {
    let started = Instant::now();
    let ext = normalize_extension(extension)?;
    debug!(
        "{LOG_PREFIX} transcribe_bytes size={} ext={ext}",
        audio_bytes.len()
    );

    let service = local_ai::global(config);

    let voice_dir = std::env::temp_dir().join("openhuman_voice_input");
    tokio::fs::create_dir_all(&voice_dir)
        .await
        .map_err(|e| format!("failed to create voice input directory: {e}"))?;

    let filename = format!(
        "voice-{}-{}.{}",
        Utc::now().timestamp_millis(),
        uuid::Uuid::new_v4(),
        ext
    );
    let file_path = voice_dir.join(filename);
    let write_started = Instant::now();
    tokio::fs::write(&file_path, audio_bytes)
        .await
        .map_err(|e| format!("failed to write audio file: {e}"))?;
    let write_elapsed = write_started.elapsed();

    let transcribe_started = Instant::now();
    // Pass context as initial_prompt to bias whisper toward known vocabulary.
    let output = service
        .transcribe_with_prompt(config, file_path.to_string_lossy().as_ref(), context)
        .await;
    let transcribe_elapsed = transcribe_started.elapsed();
    if let Err(e) = tokio::fs::remove_file(&file_path).await {
        warn!(
            "{LOG_PREFIX} failed to clean up temp audio file {}: {e}",
            file_path.display()
        );
    }

    let output = output.map_err(|e| e.to_string())?;
    let raw_text = output.text.clone();

    debug!(
        "{LOG_PREFIX} transcribe_bytes completed, text length={}, write_elapsed_ms={}, stt_elapsed_ms={}",
        raw_text.len(),
        write_elapsed.as_millis(),
        transcribe_elapsed.as_millis()
    );

    let cleanup_started = Instant::now();
    let text = if skip_cleanup {
        raw_text.clone()
    } else {
        postprocess::cleanup_transcription(config, &raw_text, context).await
    };
    let cleanup_elapsed = cleanup_started.elapsed();
    debug!(
        "{LOG_PREFIX} transcribe_bytes pipeline complete (cleanup_elapsed_ms={}, total_elapsed_ms={})",
        cleanup_elapsed.as_millis(),
        started.elapsed().as_millis()
    );

    Ok(RpcOutcome::single_log(
        VoiceSpeechResult {
            text,
            raw_text,
            model_id: output.model_id,
        },
        "voice transcription completed",
    ))
}

/// Synthesize speech from text using piper.
pub async fn voice_tts(
    config: &Config,
    text: &str,
    output_path: Option<&str>,
) -> Result<RpcOutcome<VoiceTtsResult>, String> {
    debug!(
        "{LOG_PREFIX} tts text_length={} output_path={:?}",
        text.len(),
        output_path
    );

    let service = local_ai::global(config);
    let output = service
        .tts(config, text.trim(), output_path)
        .await
        .map_err(|e| e.to_string())?;

    debug!("{LOG_PREFIX} tts completed, output={}", output.output_path);

    Ok(RpcOutcome::single_log(
        VoiceTtsResult::from(output),
        "voice tts completed",
    ))
}

/// Normalize an optional audio file extension. Returns a clean lowercase
/// alphanumeric extension string, defaulting to "webm".
pub(crate) fn normalize_extension(ext: Option<String>) -> Result<String, String> {
    let normalized = ext
        .unwrap_or_else(|| "webm".to_string())
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();

    if normalized.is_empty() {
        return Err("audio extension must not be empty".to_string());
    }
    if !normalized.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(format!(
            "invalid audio extension '{normalized}': must be alphanumeric"
        ));
    }

    Ok(normalized)
}

/// Extract the file name from an `Option<PathBuf>`, returning `"<none>"` if absent.
fn safe_basename_path(p: &Option<std::path::PathBuf>) -> String {
    p.as_ref()
        .and_then(|pb| pb.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("<none>")
        .to_string()
}

/// Extract the file name from an `Option<String>` path, returning `"<none>"` if absent.
fn safe_basename_str(p: &Option<String>) -> String {
    p.as_ref()
        .and_then(|s| std::path::Path::new(s).file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("<none>")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_extension_defaults_to_webm() {
        assert_eq!(normalize_extension(None).unwrap(), "webm");
    }

    #[test]
    fn normalize_extension_strips_dot_and_lowercases() {
        assert_eq!(
            normalize_extension(Some(".WebM".to_string())).unwrap(),
            "webm"
        );
        assert_eq!(normalize_extension(Some("OGG".to_string())).unwrap(), "ogg");
        assert_eq!(
            normalize_extension(Some("  .WAV  ".to_string())).unwrap(),
            "wav"
        );
    }

    #[test]
    fn normalize_extension_accepts_alphanumeric() {
        assert_eq!(normalize_extension(Some("m4a".to_string())).unwrap(), "m4a");
        assert_eq!(normalize_extension(Some("mp3".to_string())).unwrap(), "mp3");
    }

    #[test]
    fn normalize_extension_rejects_empty() {
        assert!(normalize_extension(Some("".to_string())).is_err());
        assert!(normalize_extension(Some("  ".to_string())).is_err());
        assert!(normalize_extension(Some(".".to_string())).is_err());
    }

    #[test]
    fn normalize_extension_rejects_invalid_chars() {
        assert!(normalize_extension(Some("a/b".to_string())).is_err());
        assert!(normalize_extension(Some("web m".to_string())).is_err());
        assert!(normalize_extension(Some("a.b".to_string())).is_err());
    }

    #[tokio::test]
    async fn voice_status_returns_without_error() {
        let config = Config::default();
        let result = voice_status(&config).await;
        assert!(result.is_ok());
        let status = result.unwrap().value;
        assert!(!status.stt_model_id.is_empty());
        assert!(!status.tts_voice_id.is_empty());
    }

    /// RAII guard that restores an env var on drop, even on panic.
    struct EnvGuard {
        key: &'static str,
        prev: Option<std::ffi::OsString>,
    }
    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let prev = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, prev }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[tokio::test]
    async fn voice_status_detects_stub_binaries() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let whisper_stub = tmp.path().join("whisper-cli");
        std::fs::write(&whisper_stub, b"#!/bin/sh\n").expect("write stub");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&whisper_stub, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }

        let _guard = EnvGuard::set("WHISPER_BIN", &whisper_stub.display().to_string());

        let mut config = Config::default();
        config.workspace_dir = tmp.path().join("workspace");
        config.config_path = tmp.path().join("config.toml");

        let result = voice_status(&config).await.unwrap();
        assert!(result.value.whisper_binary.is_some());
    }
}
