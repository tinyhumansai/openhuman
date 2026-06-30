//! STT provider implementations: cloud, local Whisper, and external (slug-keyed).

use async_trait::async_trait;
use log::{debug, warn};
use serde::Deserialize;

use super::super::cloud_transcribe::{
    transcribe_cloud, CloudTranscribeOptions, CloudTranscribeResult,
};
use super::super::local_transcribe::{transcribe_whisper, WhisperTranscribeOptions};
use super::helpers::{base64_decode, extension_for_mime};
use super::traits::{SttProvider, SttResult};
use crate::openhuman::config::schema::voice_providers::SttApiStyle;
use crate::openhuman::config::Config;
use crate::openhuman::inference::local as local_ai;
use crate::openhuman::inference::local::paths::resolve_stt_model_path_by_id;
use crate::openhuman::inference::local::whisper_engine;
use crate::openhuman::inference::local::LocalAiService;
use crate::rpc::RpcOutcome;

const LOG_PREFIX: &str = "[voice-factory]";

/// Which local-Whisper backend a request should use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WhisperRoute {
    /// Bundled in-process `whisper-rs` engine — no external binary, works
    /// for every model size. Eligible only for 16 kHz WAV input.
    InProcess,
    /// `whisper-cli` subprocess — container-aware (ffmpeg) so it handles
    /// webm/opus/mp4/ogg, but requires the external binary.
    Subprocess,
}

/// Decide the local-Whisper backend for a decoded audio blob.
///
/// In-process is preferred when enabled AND the bytes are a WAV the engine
/// can decode — this is the path that makes local STT work with no external
/// binary on a fresh install (issue #3425). Everything else (non-WAV inputs,
/// or in-process disabled) routes to the container-aware subprocess.
fn choose_whisper_route(config: &Config, audio_bytes: &[u8]) -> WhisperRoute {
    if config.local_ai.whisper_in_process && whisper_engine::looks_like_wav(audio_bytes) {
        WhisperRoute::InProcess
    } else {
        WhisperRoute::Subprocess
    }
}

/// Ensure the in-process engine has the requested model loaded, loading or
/// reloading when needed. The engine holds a single model, so a per-request
/// size change (`tiny` → `large-v3-turbo`) triggers an unload + reload so the
/// right weights are used.
///
/// **Precondition:** the caller MUST hold `service.whisper_load_lock`. That
/// same guard is then held across the subsequent transcription (see
/// [`WhisperSttProvider::try_in_process`]), so the load check, the reload, and
/// the inference form one critical section — a concurrent dispatch for a
/// different model size cannot unload/reload the engine mid-flight (which would
/// otherwise transcribe with the wrong weights or drop the request onto the
/// subprocess path).
async fn ensure_model_loaded_locked(
    service: &LocalAiService,
    config: &Config,
    model_id: &str,
) -> Result<(), String> {
    let model_path = resolve_stt_model_path_by_id(model_id, config)?;
    let target = std::path::PathBuf::from(&model_path);

    // The right model is already resident — nothing to do.
    if whisper_engine::loaded_model_path(&service.whisper).as_deref() == Some(target.as_path()) {
        return Ok(());
    }
    if whisper_engine::is_loaded(&service.whisper) {
        debug!("{LOG_PREFIX} whisper model changed; reloading for {model_id}");
        whisper_engine::unload_engine(&service.whisper);
    }

    let device = crate::openhuman::inference::device::detect_device_profile();
    let gpu = device.has_gpu;
    let gpu_desc = device.gpu_description.clone();
    let handle = service.whisper.clone();
    let model_for_load = target.clone();
    tokio::task::spawn_blocking(move || {
        whisper_engine::load_engine(&handle, &model_for_load, gpu, gpu_desc.as_deref())
    })
    .await
    .map_err(|e| format!("whisper load join error: {e}"))?
}

// ---------------------------------------------------------------------------
// Cloud STT
// ---------------------------------------------------------------------------

/// Cloud STT — wraps [`transcribe_cloud`]. Stateless; cheap to construct.
pub struct CloudSttProvider {
    model: String,
}

impl CloudSttProvider {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
        }
    }
}

#[async_trait]
impl SttProvider for CloudSttProvider {
    fn name(&self) -> &'static str {
        "cloud"
    }

    async fn transcribe(
        &self,
        config: &Config,
        audio_base64: &str,
        mime_type: Option<&str>,
        file_name: Option<&str>,
        language: Option<&str>,
    ) -> Result<RpcOutcome<SttResult>, String> {
        debug!(
            "{LOG_PREFIX} cloud STT dispatch model={} bytes_b64={}",
            self.model,
            audio_base64.len()
        );
        let opts = CloudTranscribeOptions {
            model: Some(self.model.clone()),
            language: language.map(str::to_string),
            mime_type: mime_type.map(str::to_string),
            file_name: file_name.map(str::to_string),
        };
        let outcome = transcribe_cloud(config, audio_base64, &opts).await?;
        let CloudTranscribeResult { text } = outcome.value;
        Ok(RpcOutcome::single_log(
            SttResult {
                text,
                provider: "cloud".to_string(),
            },
            "voice-factory: cloud STT completed",
        ))
    }
}

// ---------------------------------------------------------------------------
// Local Whisper STT
// ---------------------------------------------------------------------------

/// Local Whisper STT. Prefers the bundled in-process `whisper-rs` engine
/// (no external binary, every model size) for 16 kHz WAV input, falling back
/// to the [`transcribe_whisper`] `whisper-cli` subprocess for container
/// formats it can't decode (webm/opus/mp4/ogg) or when in-process is off.
pub struct WhisperSttProvider {
    model: String,
}

impl WhisperSttProvider {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
        }
    }

    /// Attempt in-process transcription. `Some(text)` on success; `None`
    /// signals the caller to fall back to the subprocess path (route not
    /// eligible, model not installed, load failure, or inference error) — we
    /// never surface an in-process error directly so a missing local binary
    /// can't turn a recoverable case into a user-facing failure.
    async fn try_in_process(
        &self,
        config: &Config,
        audio_bytes: &[u8],
        language: Option<&str>,
    ) -> Option<String> {
        if choose_whisper_route(config, audio_bytes) != WhisperRoute::InProcess {
            debug!("{LOG_PREFIX} whisper route=subprocess (non-WAV or in-process disabled)");
            return None;
        }

        let service = local_ai::global(config);

        // Hold the load lock across BOTH the load step and the transcription so
        // a concurrent dispatch for a different model size can't unload/reload
        // the single-model engine between load and inference. Transcriptions
        // already serialize on the engine handle lock, so extending the
        // critical section over the load step adds no new contention.
        let load_guard = service.whisper_load_lock.lock().await;
        if let Err(e) = ensure_model_loaded_locked(&service, config, &self.model).await {
            warn!("{LOG_PREFIX} in-process load failed ({e}); falling back to subprocess");
            return None;
        }

        let handle = service.whisper.clone();
        let bytes = audio_bytes.to_vec();
        let lang = language.map(String::from);
        let result = tokio::task::spawn_blocking(move || {
            whisper_engine::transcribe_wav_bytes(&handle, &bytes, lang.as_deref(), None)
        })
        .await;
        // Release only after inference completes — the resident model is
        // guaranteed to be `self.model` for the whole transcription above.
        drop(load_guard);

        match result {
            Ok(Ok(r)) => {
                debug!(
                    "{LOG_PREFIX} in-process whisper ok: {} chars, {}/{} segments",
                    r.text.len(),
                    r.segments_accepted,
                    r.segments_total
                );
                Some(r.text)
            }
            Ok(Err(e)) => {
                warn!(
                    "{LOG_PREFIX} in-process transcribe failed ({e}); falling back to subprocess"
                );
                None
            }
            Err(e) => {
                warn!("{LOG_PREFIX} in-process join error ({e}); falling back to subprocess");
                None
            }
        }
    }
}

#[async_trait]
impl SttProvider for WhisperSttProvider {
    fn name(&self) -> &'static str {
        "whisper"
    }

    async fn transcribe(
        &self,
        config: &Config,
        audio_base64: &str,
        mime_type: Option<&str>,
        _file_name: Option<&str>,
        language: Option<&str>,
    ) -> Result<RpcOutcome<SttResult>, String> {
        debug!(
            "{LOG_PREFIX} whisper STT dispatch model={} mime={:?} lang={:?}",
            self.model, mime_type, language
        );

        // Decode once so we can sniff the container and route. A decode
        // failure here is a genuine bad-input error worth surfacing.
        let audio_bytes = base64_decode(audio_base64)?;
        if let Some(text) = self.try_in_process(config, &audio_bytes, language).await {
            return Ok(RpcOutcome::single_log(
                SttResult {
                    text,
                    provider: "whisper".to_string(),
                },
                "voice-factory: in-process whisper STT completed",
            ));
        }

        // Fallback: container-aware whisper-cli subprocess (re-decodes the
        // base64 internally). Covers webm/opus/mp4/ogg and the in-process-off
        // case; binary resolution now also probes Homebrew dirs (issue #3425).
        let opts = WhisperTranscribeOptions {
            model: Some(self.model.clone()),
            mime_type: mime_type.map(str::to_string),
            language: language.map(str::to_string),
        };
        let outcome = transcribe_whisper(config, audio_base64, &opts).await?;
        Ok(RpcOutcome::single_log(
            SttResult {
                text: outcome.value.text,
                provider: "whisper".to_string(),
            },
            "voice-factory: whisper STT completed",
        ))
    }
}

// ---------------------------------------------------------------------------
// External STT provider (slug-keyed, third-party API)
// ---------------------------------------------------------------------------

/// Third-party STT provider dispatched via the voice provider registry.
/// Supports OpenAI-compatible and Deepgram API styles.
pub struct ExternalSttProvider {
    slug: String,
    model: String,
    endpoint: String,
    api_key: String,
    api_style: SttApiStyle,
}

impl ExternalSttProvider {
    pub fn new(
        slug: impl Into<String>,
        model: impl Into<String>,
        endpoint: impl Into<String>,
        api_key: impl Into<String>,
        api_style: SttApiStyle,
    ) -> Self {
        Self {
            slug: slug.into(),
            model: model.into(),
            endpoint: endpoint.into(),
            api_key: api_key.into(),
            api_style,
        }
    }
}

#[async_trait]
impl SttProvider for ExternalSttProvider {
    fn name(&self) -> &'static str {
        "external"
    }

    async fn transcribe(
        &self,
        _config: &Config,
        audio_base64: &str,
        mime_type: Option<&str>,
        file_name: Option<&str>,
        language: Option<&str>,
    ) -> Result<RpcOutcome<SttResult>, String> {
        debug!(
            "{LOG_PREFIX} external STT dispatch slug={} model={} style={:?} bytes_b64={}",
            self.slug,
            self.model,
            self.api_style,
            audio_base64.len()
        );

        let audio_bytes = base64_decode(audio_base64)?;
        let mime = mime_type.unwrap_or("audio/wav");

        let result = match self.api_style {
            SttApiStyle::OpenaiAudio => {
                self.transcribe_openai_compat(&audio_bytes, mime, file_name, language)
                    .await?
            }
            SttApiStyle::Deepgram => {
                self.transcribe_deepgram(&audio_bytes, mime, language)
                    .await?
            }
        };

        Ok(RpcOutcome::single_log(
            SttResult {
                text: result,
                provider: self.slug.clone(),
            },
            &format!("voice-factory: external STT completed via {}", self.slug),
        ))
    }
}

impl ExternalSttProvider {
    async fn transcribe_openai_compat(
        &self,
        audio_bytes: &[u8],
        mime: &str,
        file_name: Option<&str>,
        language: Option<&str>,
    ) -> Result<String, String> {
        let url = format!(
            "{}/audio/transcriptions",
            self.endpoint.trim_end_matches('/')
        );
        let ext = extension_for_mime(mime);
        let default_fname = format!("audio.{ext}");
        let fname = file_name.unwrap_or(&default_fname);

        let file_part = reqwest::multipart::Part::bytes(audio_bytes.to_vec())
            .file_name(fname.to_string())
            .mime_str(mime)
            .map_err(|e| format!("[voice-stt] mime error: {e}"))?;

        let mut form = reqwest::multipart::Form::new()
            .text("model", self.model.clone())
            .part("file", file_part);

        if let Some(lang) = language {
            form = form.text("language", lang.to_string());
        }

        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .multipart(form)
            .send()
            .await
            .map_err(|e| format!("[voice-stt] external STT request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("[voice-stt] external STT error {status}: {body}"));
        }

        #[derive(Deserialize)]
        struct TranscriptionResp {
            text: String,
        }
        let parsed: TranscriptionResp = resp
            .json()
            .await
            .map_err(|e| format!("[voice-stt] failed to parse response: {e}"))?;
        Ok(parsed.text)
    }

    async fn transcribe_deepgram(
        &self,
        audio_bytes: &[u8],
        mime: &str,
        language: Option<&str>,
    ) -> Result<String, String> {
        let mut url = format!(
            "{}/listen?model={}",
            self.endpoint.trim_end_matches('/'),
            self.model
        );
        if let Some(lang) = language {
            url.push_str(&format!("&language={lang}"));
        }

        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .header("Authorization", format!("Token {}", self.api_key))
            .header("Content-Type", mime)
            .body(audio_bytes.to_vec())
            .send()
            .await
            .map_err(|e| format!("[voice-stt] deepgram request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("[voice-stt] deepgram error {status}: {body}"));
        }

        #[derive(Deserialize)]
        struct DeepgramChannel {
            alternatives: Vec<DeepgramAlt>,
        }
        #[derive(Deserialize)]
        struct DeepgramAlt {
            transcript: String,
        }
        #[derive(Deserialize)]
        struct DeepgramResult {
            channels: Vec<DeepgramChannel>,
        }
        #[derive(Deserialize)]
        struct DeepgramResp {
            results: DeepgramResult,
        }

        let parsed: DeepgramResp = resp
            .json()
            .await
            .map_err(|e| format!("[voice-stt] deepgram parse error: {e}"))?;

        let text = parsed
            .results
            .channels
            .first()
            .and_then(|ch| ch.alternatives.first())
            .map(|a| a.transcript.clone())
            .unwrap_or_default();
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 12-byte RIFF/WAVE header is enough for the route sniff.
    fn wav_header() -> Vec<u8> {
        let mut w = b"RIFF".to_vec();
        w.extend_from_slice(&[0u8; 4]);
        w.extend_from_slice(b"WAVE");
        w
    }

    #[test]
    fn choose_route_in_process_for_wav_when_enabled() {
        let mut config = Config::default();
        config.local_ai.whisper_in_process = true;
        assert_eq!(
            choose_whisper_route(&config, &wav_header()),
            WhisperRoute::InProcess
        );
    }

    #[test]
    fn choose_route_subprocess_for_non_wav() {
        let mut config = Config::default();
        config.local_ai.whisper_in_process = true;
        // webm/opus magic-ish bytes → not a WAV → subprocess (ffmpeg decodes).
        assert_eq!(
            choose_whisper_route(&config, b"\x1aE\xdf\xa3 webm"),
            WhisperRoute::Subprocess
        );
    }

    #[test]
    fn choose_route_subprocess_when_in_process_disabled() {
        let mut config = Config::default();
        config.local_ai.whisper_in_process = false;
        // Even a valid WAV must go to the subprocess when in-process is off.
        assert_eq!(
            choose_whisper_route(&config, &wav_header()),
            WhisperRoute::Subprocess
        );
    }
}
