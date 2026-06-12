//! Factory functions for creating voice (STT / TTS) providers.
//!
//! Mirrors the shape of [`crate::openhuman::embeddings::factory`]: a single
//! entry point that takes a provider name + parameters and returns a boxed
//! trait object. Production paths pick the provider based on the user's
//! config (`stt_provider`, `tts_provider`); unit tests use the factory
//! directly to verify dispatch branches.
//!
//! ## Provider-string grammar
//!
//! Mirrors the LLM inference factory pattern in
//! [`crate::openhuman::inference::provider::factory`]:
//!
//! | String                | Resolves to                                    |
//! |-----------------------|------------------------------------------------|
//! | `"cloud"` / `"openhuman"` | OpenHuman backend proxy                    |
//! | `"whisper"`           | Local Whisper (STT)                            |
//! | `"piper"`             | Local Piper (TTS)                              |
//! | `"<slug>:<model>"`    | Voice provider entry matched by slug           |
//! | `"<slug>"`            | Bare slug — uses provider's default model/voice|
//!
//! ## STT providers
//!
//! - `"cloud"` → backend Whisper proxy (POST `/openai/v1/audio/transcriptions`).
//! - `"whisper"` → local Whisper via `WHISPER_BIN` (or in-process `whisper-rs`).
//! - `"<slug>:<model>"` → third-party STT API via the voice provider registry
//!   (e.g. `"deepgram:nova-2"`, `"openai:whisper-1"`).
//!
//! ## TTS providers
//!
//! - `"cloud"` → backend ElevenLabs proxy (POST `/openai/v1/audio/speech`)
//!   which also returns Oculus-15 visemes for the mascot lip-sync.
//! - `"piper"` → local Piper subprocess via `PIPER_BIN`.
//! - `"<slug>:<voice>"` → third-party TTS API via the voice provider registry
//!   (e.g. `"openai:alloy"`, `"elevenlabs:<voice_id>"`).
//!
//! ## Logging prefixes
//!
//! All factory branches log against `[voice-factory]`; the wrapped provider
//! implementations log under `[voice-stt]` / `[voice-tts]` so end-to-end
//! traces grep cleanly.

mod entry;
mod helpers;
mod stt_providers;
mod traits;
mod tts_providers;

#[cfg(test)]
mod tests;

// Re-export the public API — exact visibility preserved from the original file.
pub use entry::{
    create_stt_provider, create_tts_provider, default_stt_provider, default_tts_provider,
    DEFAULT_PIPER_VOICE, DEFAULT_WHISPER_MODEL, WHISPER_MODEL_PRESETS,
};
pub use helpers::{effective_stt_provider, effective_tts_provider};
pub use stt_providers::{CloudSttProvider, ExternalSttProvider, WhisperSttProvider};
pub use traits::{SttProvider, SttResult, TtsProvider};
pub use tts_providers::{CloudTtsProvider, ExternalTtsProvider, PiperTtsProvider};
