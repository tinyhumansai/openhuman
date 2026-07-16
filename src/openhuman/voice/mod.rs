//! Voice domain — speech-to-text (whisper.cpp) and text-to-speech (piper).
//!
//! Provides RPC endpoints under the `openhuman.voice_*` namespace for
//! transcription, synthesis, proactive availability checking, and a
//! standalone voice dictation server (hotkey → record → transcribe → insert).
//!
//! Inference implementations (local_speech, local_transcribe, cloud_transcribe,
//! hallucination, streaming, postprocess) now live under
//! `crate::openhuman::inference::voice` so all inference concerns share a
//! single domain root.
//!
//! ## Compile-time gate (`voice` feature)
//!
//! `pub mod voice;` is ALWAYS compiled — it is a facade. The real
//! implementation (the submodules below and the `inference::voice` re-exports)
//! is gated behind the default-ON `voice` Cargo feature. When the feature is
//! off, [`stub`] takes its place and exposes the same public surface that
//! always-on / other-gated callers depend on (`server`, `dictation_listener`,
//! `streaming`, `reply_speech`, `cloud_transcribe`, `create_stt_provider`,
//! `effective_stt_provider`, `publish_ptt_transcript_committed`) with
//! no-op / disabled-error bodies. Keeping the two surfaces in lockstep is
//! enforced by the disabled-build check
//! (`cargo check --no-default-features --features "<all-but-voice>"`): any
//! signature drift fails that build.

// Ungated — part of the always-compiled facade (see the module docs above): it
// reports which side of the gate this binary landed on, so it must exist in
// both states.
pub mod compile_status;
pub use compile_status::VOICE_COMPILED_IN;

#[cfg(feature = "voice")]
pub mod always_on;
#[cfg(feature = "voice")]
pub mod audio_capture;
#[cfg(feature = "voice")]
pub mod bus;
#[cfg(feature = "voice")]
pub use bus::publish_ptt_transcript_committed;
#[cfg(feature = "voice")]
pub(crate) mod cli;
#[cfg(feature = "voice")]
pub mod command_router;
#[cfg(feature = "voice")]
pub mod dictation_listener;
#[cfg(feature = "voice")]
pub mod factory;
#[cfg(feature = "voice")]
pub mod hotkey;
#[cfg(feature = "voice")]
mod ops;
#[cfg(feature = "voice")]
pub mod reply_speech;
#[cfg(feature = "voice")]
mod schemas;
#[cfg(feature = "voice")]
pub mod server;
#[cfg(feature = "voice")]
pub mod text_input;
#[cfg(feature = "voice")]
mod types;

// Re-export the inference-side voice modules so `voice::local_speech`,
// `voice::local_transcribe`, etc. continue to resolve for existing callers.
#[cfg(feature = "voice")]
pub use crate::openhuman::inference::voice::cloud_transcribe;
#[cfg(feature = "voice")]
pub use crate::openhuman::inference::voice::hallucination;
#[cfg(feature = "voice")]
pub use crate::openhuman::inference::voice::local_speech;
#[cfg(feature = "voice")]
pub use crate::openhuman::inference::voice::local_transcribe;
#[cfg(feature = "voice")]
pub use crate::openhuman::inference::voice::postprocess;
#[cfg(feature = "voice")]
pub use crate::openhuman::inference::voice::streaming;

#[cfg(feature = "voice")]
pub use factory::{
    create_stt_provider, create_tts_provider, default_stt_provider, default_tts_provider,
    effective_stt_provider, effective_tts_provider, ExternalSttProvider, ExternalTtsProvider,
    SttProvider, SttResult, TtsProvider, DEFAULT_PIPER_VOICE, DEFAULT_WHISPER_MODEL,
    WHISPER_MODEL_PRESETS,
};
#[cfg(feature = "voice")]
pub use ops::*;
#[cfg(feature = "voice")]
pub use schemas::{all_voice_controller_schemas, all_voice_registered_controllers, voice_schemas};
#[cfg(feature = "voice")]
pub use types::{VoiceSpeechResult, VoiceStatus, VoiceTtsResult};

/// Default Whisper-v1 model id sent to the backend cloud STT proxy. Kept
/// here (rather than in `cloud_transcribe.rs`) so the factory module can
/// reach it via the public `voice::` surface without re-exporting an
/// internal constant.
#[cfg(feature = "voice")]
pub(crate) fn cloud_transcribe_default_model() -> &'static str {
    "whisper-v1"
}

// ---------------------------------------------------------------------------
// Disabled facade — compiled only when the `voice` feature is OFF.
// ---------------------------------------------------------------------------

#[cfg(not(feature = "voice"))]
mod stub;
#[cfg(not(feature = "voice"))]
pub use stub::*;
