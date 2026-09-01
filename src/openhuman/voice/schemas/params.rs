//! Param structs for voice controller RPC handlers.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(super) struct TranscribeParams {
    pub(super) audio_path: String,
    /// Optional conversation context for LLM post-processing.
    #[serde(default)]
    pub(super) context: Option<String>,
    /// Skip LLM cleanup and return the raw engine output.
    #[serde(default)]
    pub(super) skip_cleanup: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct TranscribeBytesParams {
    pub(super) audio_bytes: Vec<u8>,
    #[serde(default)]
    pub(super) extension: Option<String>,
    /// Optional conversation context for LLM post-processing.
    #[serde(default)]
    pub(super) context: Option<String>,
    /// Skip LLM cleanup and return the raw engine output.
    #[serde(default)]
    pub(super) skip_cleanup: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct TtsParams {
    pub(super) text: String,
    #[serde(default)]
    pub(super) output_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CloudTranscribeParams {
    pub(super) audio_base64: String,
    #[serde(default)]
    pub(super) mime_type: Option<String>,
    #[serde(default)]
    pub(super) file_name: Option<String>,
    #[serde(default)]
    pub(super) model: Option<String>,
    #[serde(default)]
    pub(super) language: Option<String>,
}

/// Factory-dispatched STT request. The caller can either pin a provider
/// explicitly (`"cloud"` / `"<slug>[:<model>]"`) or let the controller resolve the
/// effective provider from `config.local_ai.stt_provider`. Keeps the
/// existing `voice_cloud_transcribe` RPC intact for back-compat — older
/// renderers still pin the cloud path directly.
#[derive(Debug, Deserialize)]
pub(super) struct SttDispatchParams {
    pub(super) audio_base64: String,
    /// Provider override; falls back to `config.local_ai.stt_provider`.
    #[serde(default)]
    pub(super) provider: Option<String>,
    /// Model override (cloud branch ignores it).
    #[serde(default)]
    pub(super) model: Option<String>,
    #[serde(default)]
    pub(super) mime_type: Option<String>,
    #[serde(default)]
    pub(super) file_name: Option<String>,
    #[serde(default)]
    pub(super) language: Option<String>,
}

/// Factory-dispatched TTS request. Same provider-resolution rule as
/// [`SttDispatchParams`].
#[derive(Debug, Deserialize)]
pub(super) struct TtsDispatchParams {
    pub(super) text: String,
    #[serde(default)]
    pub(super) provider: Option<String>,
    #[serde(default)]
    pub(super) voice: Option<String>,
}

/// Settings-panel update for the STT/TTS provider selectors. Both are
/// optional; omitted fields are left at their current value.
#[derive(Debug, Deserialize)]
pub(super) struct SetProvidersParams {
    #[serde(default)]
    pub(super) stt_provider: Option<String>,
    #[serde(default)]
    pub(super) tts_provider: Option<String>,
    #[serde(default)]
    pub(super) stt_model: Option<String>,
    #[serde(default)]
    pub(super) tts_voice: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ReplySynthesizeParams {
    pub(super) text: String,
    #[serde(default)]
    pub(super) voice_id: Option<String>,
    #[serde(default)]
    pub(super) model_id: Option<String>,
    #[serde(default)]
    pub(super) output_format: Option<String>,
}

/// Voice provider registry update. Mirrors `InferenceUpdateModelSettingsParams`.
#[derive(Debug, Deserialize)]
pub(super) struct VoiceUpdateProviderSettingsParams {
    #[serde(default)]
    pub(super) voice_providers: Option<Vec<VoiceProviderCredUpdate>>,
    #[serde(default)]
    pub(super) stt_provider: Option<String>,
    #[serde(default)]
    pub(super) tts_provider: Option<String>,
}

/// Wire format for a single voice provider entry in the update call.
#[derive(Debug, Deserialize)]
pub(super) struct VoiceProviderCredUpdate {
    #[serde(default)]
    pub(super) id: Option<String>,
    pub(super) slug: String,
    #[serde(default)]
    pub(super) label: Option<String>,
    #[serde(default)]
    pub(super) endpoint: Option<String>,
    #[serde(default)]
    pub(super) auth_style: Option<String>,
    #[serde(default)]
    pub(super) capability: Option<String>,
    #[serde(default)]
    pub(super) stt_api_style: Option<String>,
    #[serde(default)]
    pub(super) tts_api_style: Option<String>,
    #[serde(default)]
    pub(super) default_stt_model: Option<String>,
    #[serde(default)]
    pub(super) default_tts_voice: Option<String>,
}

/// List models/voices for a voice provider.
#[derive(Debug, Deserialize)]
pub(super) struct VoiceListModelsParams {
    pub(super) provider_id: String,
    #[serde(default)]
    pub(super) capability: Option<String>,
}

/// Test a voice provider endpoint.
///
/// `Debug` is implemented by hand rather than derived: `api_key` carries a raw
/// user credential, and a derived `Debug` would print it in full the first time
/// anyone adds a `{:?}` of this struct to a log line.
#[derive(Deserialize)]
pub(super) struct VoiceTestProviderParams {
    pub(super) workload: String,
    pub(super) provider: String,
    /// When true, only validate the API key (lightweight GET) without
    /// synthesizing or transcribing. Used by the provider-enable modal.
    #[serde(default)]
    pub(super) validate_only: bool,
    /// Candidate API key to validate **without storing it**, so the
    /// provider-enable modal can check a key before the user commits to
    /// saving it (#5896). Omitted means "use the stored credential for this
    /// provider's slug". Only consulted when `validate_only` is set.
    #[serde(default)]
    pub(super) api_key: Option<String>,
}

impl std::fmt::Debug for VoiceTestProviderParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceTestProviderParams")
            .field("workload", &self.workload)
            .field("provider", &self.provider)
            .field("validate_only", &self.validate_only)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum OverlaySttState {
    RecordingStarted,
    TranscriptionDone,
    Cancelled,
    Error,
}

#[derive(Debug, Deserialize)]
pub(super) struct OverlaySttNotifyParams {
    /// Voice state transition.
    pub(super) state: OverlaySttState,
    /// Transcribed text (required when state is "transcription_done").
    #[serde(default)]
    pub(super) text: Option<String>,
}
