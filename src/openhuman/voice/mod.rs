//! Voice domain — speech-to-text (whisper.cpp) and text-to-speech (piper).
//!
//! Provides RPC endpoints under the `openhuman.voice_*` namespace for
//! transcription, synthesis, proactive availability checking, and a
//! standalone voice dictation server (hotkey → record → transcribe → insert).

pub mod audio_capture;
pub(crate) mod cli;
pub mod cloud_transcribe;
pub mod dictation_listener;
pub mod factory;
pub mod hallucination;
pub mod hotkey;
pub mod local_speech;
pub mod local_transcribe;
mod ops;
mod postprocess;
pub mod reply_speech;
mod schemas;
pub mod server;
pub mod streaming;
pub mod text_input;
mod types;

pub use factory::{
    create_stt_provider, create_tts_provider, default_stt_provider, default_tts_provider,
    SttProvider, SttResult, TtsProvider, DEFAULT_PIPER_VOICE, DEFAULT_WHISPER_MODEL,
    WHISPER_MODEL_PRESETS,
};
pub use ops::*;
pub use schemas::{all_voice_controller_schemas, all_voice_registered_controllers, voice_schemas};
pub use types::{VoiceSpeechResult, VoiceStatus, VoiceTtsResult};

/// Default Whisper-v1 model id sent to the backend cloud STT proxy. Kept
/// here (rather than in `cloud_transcribe.rs`) so the factory module can
/// reach it via the public `voice::` surface without re-exporting an
/// internal constant.
pub(crate) fn cloud_transcribe_default_model() -> &'static str {
    "whisper-v1"
}
