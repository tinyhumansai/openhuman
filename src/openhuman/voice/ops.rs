//! Voice domain business logic — STT (whisper.cpp) and TTS (piper).
//!
//! Each public function follows the `RpcOutcome<T>` pattern used by other
//! domain modules (billing, health, etc.).

use chrono::Utc;
use log::debug;

use crate::openhuman::config::Config;
use crate::openhuman::local_ai;
use crate::openhuman::local_ai::model_ids;
use crate::openhuman::local_ai::paths::{
    resolve_piper_binary, resolve_stt_model_path, resolve_tts_voice_path, resolve_whisper_binary,
};
use crate::rpc::RpcOutcome;

use super::types::{VoiceSpeechResult, VoiceStatus, VoiceTtsResult};

const LOG_PREFIX: &str = "[voice]";

/// Check availability of STT/TTS binaries and models without executing them.
pub async fn voice_status(config: &Config) -> Result<RpcOutcome<VoiceStatus>, String> {
    debug!("{LOG_PREFIX} checking voice status");

    let whisper_bin = resolve_whisper_binary();
    let piper_bin = resolve_piper_binary();
    let stt_model = resolve_stt_model_path(config).ok();
    let tts_voice = resolve_tts_voice_path(config).ok();

    let stt_available = whisper_bin.is_some() && stt_model.is_some();
    let tts_available = piper_bin.is_some() && tts_voice.is_some();

    debug!(
        "{LOG_PREFIX} stt_available={stt_available} tts_available={tts_available} \
         whisper_bin={:?} piper_bin={:?} stt_model={:?} tts_voice={:?}",
        whisper_bin, piper_bin, stt_model, tts_voice
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
    };

    Ok(RpcOutcome::single_log(status, "voice status checked"))
}

/// Transcribe audio from a file path using whisper.cpp.
pub async fn voice_transcribe(
    config: &Config,
    audio_path: &str,
) -> Result<RpcOutcome<VoiceSpeechResult>, String> {
    debug!("{LOG_PREFIX} transcribing audio_path={audio_path}");

    let service = local_ai::global(config);
    let output = service
        .transcribe(config, audio_path.trim())
        .await
        .map_err(|e| e.to_string())?;

    debug!("{LOG_PREFIX} transcription completed, text length={}", output.text.len());

    Ok(RpcOutcome::single_log(
        VoiceSpeechResult::from(output),
        "voice transcription completed",
    ))
}

/// Transcribe audio from raw bytes. Writes to a temp file, transcribes, cleans up.
pub async fn voice_transcribe_bytes(
    config: &Config,
    audio_bytes: &[u8],
    extension: Option<String>,
) -> Result<RpcOutcome<VoiceSpeechResult>, String> {
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
    tokio::fs::write(&file_path, audio_bytes)
        .await
        .map_err(|e| format!("failed to write audio file: {e}"))?;

    let output = service
        .transcribe(config, file_path.to_string_lossy().as_ref())
        .await;
    let _ = tokio::fs::remove_file(&file_path).await;

    let output = output.map_err(|e| e.to_string())?;

    debug!(
        "{LOG_PREFIX} transcribe_bytes completed, text length={}",
        output.text.len()
    );

    Ok(RpcOutcome::single_log(
        VoiceSpeechResult::from(output),
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
        assert_eq!(
            normalize_extension(Some("OGG".to_string())).unwrap(),
            "ogg"
        );
        assert_eq!(
            normalize_extension(Some("  .WAV  ".to_string())).unwrap(),
            "wav"
        );
    }

    #[test]
    fn normalize_extension_accepts_alphanumeric() {
        assert_eq!(
            normalize_extension(Some("m4a".to_string())).unwrap(),
            "m4a"
        );
        assert_eq!(
            normalize_extension(Some("mp3".to_string())).unwrap(),
            "mp3"
        );
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
        // Without binaries installed, both should be false
        // but the function itself should not error
        assert!(!status.stt_model_id.is_empty());
        assert!(!status.tts_voice_id.is_empty());
    }

    #[tokio::test]
    async fn voice_status_detects_stub_binaries() {
        let tmp = tempfile::tempdir().expect("tempdir");

        // Create a stub whisper-cli binary
        let whisper_stub = tmp.path().join("whisper-cli");
        std::fs::write(&whisper_stub, b"#!/bin/sh\n").expect("write stub");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&whisper_stub, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }

        // Set WHISPER_BIN to point at the stub
        std::env::set_var("WHISPER_BIN", whisper_stub.display().to_string());

        let mut config = Config::default();
        config.workspace_dir = tmp.path().join("workspace");
        config.config_path = tmp.path().join("config.toml");

        let result = voice_status(&config).await.unwrap();
        assert!(result.value.whisper_binary.is_some());

        // Clean up env
        std::env::remove_var("WHISPER_BIN");
    }
}
