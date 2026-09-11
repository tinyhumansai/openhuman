use std::path::PathBuf;
use std::time::Instant;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use log::debug;

use crate::openhuman::config::Config;
use crate::openhuman::inference::model_ids;
use crate::openhuman::inference::paths::{
    config_root_dir, resolve_piper_binary, resolve_tts_voice_path,
};
use crate::openhuman::inference::types::{LocalAiSpeechResult, LocalAiTtsResult};
use crate::openhuman::voice::{create_stt_provider, effective_stt_provider};

use super::LocalAiService;

const LOG_PREFIX: &str = "[speech]";

/// MIME hint sent to the backend for a given file extension. The backend
/// forwards the blob to its STT provider, which sniffs the container; a wrong
/// hint only costs a re-sniff, so an unknown extension falls back to WAV.
fn mime_for_extension(ext: &str) -> &'static str {
    match ext {
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "m4a" | "mp4" => "audio/mp4",
        "ogg" | "opus" => "audio/ogg",
        "webm" => "audio/webm",
        "flac" => "audio/flac",
        _ => "audio/wav",
    }
}

impl LocalAiService {
    pub async fn transcribe(
        &self,
        config: &Config,
        audio_path: &str,
    ) -> Result<LocalAiSpeechResult, String> {
        self.transcribe_with_prompt(config, audio_path, None).await
    }

    /// Transcribe an audio file on disk.
    ///
    /// **No longer local.** The bundled whisper.cpp engine (in-process
    /// `whisper-rs` and the `whisper-cli` subprocess) was removed along with
    /// its model downloader. Dispatch through the configured hosted STT engine
    /// (`voice_server.stt_engine` or an explicit provider override), so every
    /// dictation entry point honors the user's selected provider.
    ///
    /// `initial_prompt` (the custom-dictionary vocabulary bias) is accepted for
    /// signature compatibility and **ignored**: the backend transcription
    /// endpoint exposes no prompt field. It is logged so a user wondering why
    /// their dictionary stopped biasing results can find the reason.
    pub async fn transcribe_with_prompt(
        &self,
        config: &Config,
        audio_path: &str,
        initial_prompt: Option<&str>,
    ) -> Result<LocalAiSpeechResult, String> {
        let started = Instant::now();
        if let Some(prompt) = initial_prompt.filter(|p| !p.trim().is_empty()) {
            debug!(
                "{LOG_PREFIX} initial_prompt ({} chars) ignored — the hosted STT endpoint has no \
                 prompt/vocabulary-bias parameter",
                prompt.len()
            );
        }

        let path = std::path::Path::new(audio_path);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let mime = mime_for_extension(&ext);
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("audio.wav")
            .to_string();

        let provider_name = effective_stt_provider(config);
        debug!(
            "{LOG_PREFIX} configured STT dispatch provider={provider_name} path={audio_path} mime={mime}"
        );
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|e| format!("failed to read audio file {audio_path}: {e}"))?;
        if bytes.is_empty() {
            return Err(format!("audio file {audio_path} is empty"));
        }
        let audio_base64 = BASE64.encode(&bytes);

        let provider =
            create_stt_provider(&provider_name, "", config).map_err(|error| error.to_string())?;
        let outcome = provider
            .transcribe(config, &audio_base64, Some(mime), Some(&file_name), None)
            .await?;
        debug!(
            "{LOG_PREFIX} configured STT complete (provider={} bytes={} elapsed_ms={})",
            outcome.value.provider,
            bytes.len(),
            started.elapsed().as_millis()
        );
        self.status.lock().stt_state = "ready".to_string();
        Ok(LocalAiSpeechResult {
            text: outcome.value.text,
            model_id: model_ids::effective_stt_model_id(config),
        })
    }

    pub async fn tts(
        &self,
        config: &Config,
        text: &str,
        output_path: Option<&str>,
    ) -> Result<LocalAiTtsResult, String> {
        if !config.local_ai.runtime_enabled {
            return Err("local ai is disabled".to_string());
        }
        let piper_bin = resolve_piper_binary()
            .ok_or_else(|| "piper binary not found. Set PIPER_BIN or install piper.".to_string())?;
        let model_path = resolve_tts_voice_path(config)?;
        let out_path = output_path
            .map(std::string::ToString::to_string)
            .unwrap_or_else(|| {
                config_root_dir(config)
                    .join("models")
                    .join("local-ai")
                    .join("tts-output.wav")
                    .display()
                    .to_string()
            });
        let parent = PathBuf::from(&out_path)
            .parent()
            .map(PathBuf::from)
            .ok_or_else(|| "invalid output_path".to_string())?;
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("failed to create TTS output directory: {e}"))?;

        let mut child = tokio::process::Command::new(piper_bin)
            .args(["--model", &model_path, "--output_file", &out_path])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to launch piper: {e}"))?;

        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin
                .write_all(text.as_bytes())
                .await
                .map_err(|e| format!("failed to write text to piper stdin: {e}"))?;
        }
        let output = child
            .wait_with_output()
            .await
            .map_err(|e| format!("failed to wait for piper: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "piper failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        self.status.lock().tts_state = "ready".to_string();
        Ok(LocalAiTtsResult {
            output_path: out_path,
            voice_id: model_ids::effective_tts_voice_id(config),
        })
    }
}
