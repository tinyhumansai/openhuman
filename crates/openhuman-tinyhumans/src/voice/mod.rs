//! Hosted speech: cloud transcription, reply speech, realtime voice agents and the hosted live providers.
//!
//! Everything here plugs into the core through the generic registries on
//! `openhuman_embed::__host::voice_registry`; the core holds no hosted voice
//! route. [`register`] is the one place it does so; `crate::register::register_all`
//! calls it from `install()`.
//!
//! - [`cloud_transcribe`]: `/openai/v1/audio/transcriptions` (STT).
//! - [`reply_speech`]: `/openai/v1/audio/speech` (TTS with visemes).
//! - [`realtime`]: `/voice-agent/get-signed-url` (ElevenLabs agent sessions) and
//!   the `voice.agent_signed_url` controller.
//! - [`speech_providers`]: the STT/TTS providers registered under the
//!   `cloud` / `openhuman` / `backend` slugs.
//! - [`schemas`]: the `voice.agent_signed_url` and `voice.cloud_transcribe` controllers.
//! - [`live`]: the `gemini-hosted` and `elevenlabs-hosted` live providers.

pub mod cloud_transcribe;
pub mod live;
pub mod realtime;
pub mod reply_speech;
pub mod schemas;
pub mod speech_providers;

use crate::install::InstallOptions;

/// Registers this area's hooks with the core. Idempotent.
pub(crate) fn register(_options: &InstallOptions) {
    log::debug!("[tinyhumans] voice: registering hosted STT/TTS and live providers");
    speech_providers::register();
    live::register();
    schemas::register();
}
