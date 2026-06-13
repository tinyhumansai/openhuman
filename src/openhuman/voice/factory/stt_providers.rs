//! STT provider implementations: cloud, local Whisper, and external (slug-keyed).

use async_trait::async_trait;
use log::debug;
use serde::Deserialize;

use super::super::cloud_transcribe::{
    transcribe_cloud, CloudTranscribeOptions, CloudTranscribeResult,
};
use super::super::local_transcribe::{transcribe_whisper, WhisperTranscribeOptions};
use super::helpers::{base64_decode, extension_for_mime};
use super::traits::{SttProvider, SttResult};
use crate::openhuman::config::schema::voice_providers::SttApiStyle;
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

const LOG_PREFIX: &str = "[voice-factory]";

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

/// Local Whisper STT — wraps [`transcribe_whisper`]. Resolves `WHISPER_BIN`
/// lazily on each call.
pub struct WhisperSttProvider {
    model: String,
}

impl WhisperSttProvider {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
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
