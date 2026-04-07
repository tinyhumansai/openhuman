use std::path::PathBuf;
use std::time::Instant;

use log::{debug, warn};

use crate::openhuman::config::Config;
use crate::openhuman::local_ai::model_ids;
use crate::openhuman::local_ai::paths::{
    config_root_dir, resolve_piper_binary, resolve_stt_model_path, resolve_tts_voice_path,
    resolve_whisper_binary,
};
use crate::openhuman::local_ai::types::{LocalAiSpeechResult, LocalAiTtsResult};

use super::whisper_engine;
use super::LocalAiService;

const LOG_PREFIX: &str = "[speech]";

impl LocalAiService {
    pub async fn transcribe(
        &self,
        config: &Config,
        audio_path: &str,
    ) -> Result<LocalAiSpeechResult, String> {
        self.transcribe_with_prompt(config, audio_path, None).await
    }

    /// Transcribe audio with an optional initial_prompt for vocabulary bias.
    ///
    /// The `initial_prompt` is passed to whisper.cpp's `initial_prompt` parameter,
    /// biasing the decoder toward the supplied words/phrases. Used for custom
    /// dictionary support and conversational continuity.
    pub async fn transcribe_with_prompt(
        &self,
        config: &Config,
        audio_path: &str,
        initial_prompt: Option<&str>,
    ) -> Result<LocalAiSpeechResult, String> {
        let started = Instant::now();
        if !config.local_ai.enabled {
            return Err("local ai is disabled".to_string());
        }

        // Lazily load in-process whisper engine when enabled. Serialize load attempts
        // so concurrent requests do not spawn duplicate heavy contexts.
        if config.local_ai.whisper_in_process && !whisper_engine::is_loaded(&self.whisper) {
            let lazy_load_started = Instant::now();
            let _load_guard = self.whisper_load_lock.lock().await;
            if !whisper_engine::is_loaded(&self.whisper) {
                if let Ok(model_path) = resolve_stt_model_path(config) {
                    let handle = self.whisper.clone();
                    let model = std::path::PathBuf::from(model_path);
                    debug!(
                        "{LOG_PREFIX} whisper in-process enabled but unloaded; loading model lazily"
                    );
                    let load_result = tokio::task::spawn_blocking(move || {
                        whisper_engine::load_engine(&handle, &model)
                    })
                    .await
                    .map_err(|e| format!("whisper load task join error: {e}"))?;
                    if let Err(e) = load_result {
                        warn!("{LOG_PREFIX} lazy in-process whisper load failed: {e}");
                    } else {
                        debug!(
                            "{LOG_PREFIX} lazy in-process whisper load complete (elapsed_ms={})",
                            lazy_load_started.elapsed().as_millis()
                        );
                    }
                } else {
                    debug!(
                        "{LOG_PREFIX} lazy in-process load skipped: STT model path not resolved"
                    );
                }
            }
        }

        // Try in-process whisper engine first (offloaded to a blocking thread).
        if whisper_engine::is_loaded(&self.whisper) {
            debug!("{LOG_PREFIX} using in-process whisper engine for {audio_path}");
            let handle = self.whisper.clone();
            let path = audio_path.to_string();
            let prompt_owned = initial_prompt.map(String::from);
            let in_process_started = Instant::now();
            let result = tokio::task::spawn_blocking(move || {
                Self::transcribe_in_process_inner(&handle, &path, prompt_owned.as_deref())
            })
            .await
            .map_err(|e| format!("whisper task join error: {e}"))?;
            match result {
                Ok(text) => {
                    debug!(
                        "{LOG_PREFIX} in-process transcription complete (elapsed_ms={}, total_elapsed_ms={})",
                        in_process_started.elapsed().as_millis(),
                        started.elapsed().as_millis()
                    );
                    self.status.lock().stt_state = "ready".to_string();
                    return Ok(LocalAiSpeechResult {
                        text,
                        model_id: model_ids::effective_stt_model_id(config),
                    });
                }
                Err(e) => {
                    warn!("{LOG_PREFIX} in-process transcription failed, falling back to CLI: {e}");
                }
            }
        }

        // Fallback: subprocess per call (original behavior).
        debug!("{LOG_PREFIX} using whisper-cli subprocess for {audio_path}");
        let subprocess_started = Instant::now();
        let result = self.transcribe_subprocess(config, audio_path).await;
        debug!(
            "{LOG_PREFIX} subprocess transcription finished (elapsed_ms={}, total_elapsed_ms={})",
            subprocess_started.elapsed().as_millis(),
            started.elapsed().as_millis()
        );
        result
    }

    /// Transcribe using the in-process whisper-rs engine. Runs on a blocking
    /// thread — takes the engine handle directly so it can be `Send`.
    fn transcribe_in_process_inner(
        handle: &whisper_engine::WhisperEngineHandle,
        audio_path: &str,
        initial_prompt: Option<&str>,
    ) -> Result<String, String> {
        let path = std::path::Path::new(audio_path);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        let result = if ext == "wav" {
            whisper_engine::transcribe_wav_file(handle, path, None, initial_prompt)?
        } else {
            warn!(
                "{LOG_PREFIX} non-WAV input ({ext}), attempting WAV decode anyway \
                 (may fail — use ffmpeg conversion for best results)"
            );
            whisper_engine::transcribe_wav_file(handle, path, None, initial_prompt)?
        };

        debug!(
            "{LOG_PREFIX} in-process result: avg_logprob={:.3}, segments={}/{}",
            result.avg_logprob.unwrap_or(0.0),
            result.segments_accepted,
            result.segments_total
        );
        Ok(result.text)
    }

    /// Original subprocess-based transcription via whisper-cli.
    async fn transcribe_subprocess(
        &self,
        config: &Config,
        audio_path: &str,
    ) -> Result<LocalAiSpeechResult, String> {
        let whisper_bin = resolve_whisper_binary().ok_or_else(|| {
            "whisper.cpp binary not found. Set WHISPER_BIN or install whisper-cli.".to_string()
        })?;
        let model_path = resolve_stt_model_path(config)?;
        let output = tokio::process::Command::new(whisper_bin)
            .args(["-m", &model_path, "-f", audio_path])
            .output()
            .await
            .map_err(|e| format!("failed to run whisper.cpp: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "whisper.cpp failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if text.is_empty() {
            return Err("whisper.cpp returned empty transcript".to_string());
        }
        self.status.lock().stt_state = "ready".to_string();
        Ok(LocalAiSpeechResult {
            text,
            model_id: model_ids::effective_stt_model_id(config),
        })
    }

    pub async fn tts(
        &self,
        config: &Config,
        text: &str,
        output_path: Option<&str>,
    ) -> Result<LocalAiTtsResult, String> {
        if !config.local_ai.enabled {
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
