//! STT provider implementation: external (slug-keyed) third-party APIs. The
//! hosted backend proxy is registered by the hosted-backend layer.
//!
//! There is no local branch any more. The bundled whisper.cpp engine — both the
//! in-process `whisper-rs` context and the `whisper-cli` subprocess fallback —
//! was removed along with its model downloader; every path here is an HTTP call.

use async_trait::async_trait;
use log::debug;

use super::helpers::base64_decode;
use super::traits::{SttProvider, SttResult};
use crate::config::Config;
use crate::core::Outcome;
use tinyinference_voice::external_stt::{ExternalSttClient, SttApiStyle};

const LOG_PREFIX: &str = "[voice-factory]";

// ---------------------------------------------------------------------------
// External STT provider (slug-keyed, third-party API)
// ---------------------------------------------------------------------------

/// Third-party STT provider dispatched via the voice provider registry.
/// Supports OpenAI-compatible, Deepgram, and ElevenLabs API styles; the HTTP
/// clients live in `tinyinference_voice::external_stt`.
pub struct ExternalSttProvider {
    slug: String,
    api_style: SttApiStyle,
    client: ExternalSttClient,
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
            api_style,
            client: ExternalSttClient::new(
                reqwest::Client::new(),
                model,
                endpoint,
                api_key,
                api_style,
            ),
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
    ) -> Result<Outcome<SttResult>, String> {
        debug!(
            "{LOG_PREFIX} external STT dispatch slug={} model={} style={:?} bytes_b64={}",
            self.slug,
            self.client.model(),
            self.api_style,
            audio_base64.len()
        );

        let audio_bytes = base64_decode(audio_base64)?;
        let mime = mime_type.unwrap_or("audio/wav");
        let text = self
            .client
            .transcribe(&audio_bytes, mime, file_name, language)
            .await?;

        Ok(Outcome::single_log(
            SttResult {
                text,
                provider: self.slug.clone(),
            },
            format!("voice-factory: external STT completed via {}", self.slug),
        ))
    }

    #[cfg(test)]
    fn configured_model(&self) -> Option<&str> {
        Some(self.client.model())
    }
}

#[cfg(test)]
#[path = "stt_providers_tests.rs"]
mod tests;
