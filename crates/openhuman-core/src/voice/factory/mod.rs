//! Factory functions for creating voice (STT / TTS) providers.
//!
//! Mirrors the shape of [`crate::inference::embedding_host::factory`]: a single
//! entry point that takes a provider name + parameters and returns a boxed
//! trait object. Production paths pick the provider based on the user's
//! config (`stt_provider`, `tts_provider`); unit tests use the factory
//! directly to verify dispatch branches.
//!
//! ## Provider-string grammar
//!
//! Mirrors the LLM inference factory pattern in
//! [`crate::inference::provider::factory`]:
//!
//! | String                | Resolves to                                    |
//! |-----------------------|------------------------------------------------|
//! | `"cloud"` / `"openhuman"` / `"backend"` | a registered hosted provider |
//! | `"piper"`             | Local Piper (TTS)                              |
//! | `"<slug>:<model>"`    | Voice provider entry matched by slug           |
//! | `"<slug>"`            | Bare slug — uses provider's default model/voice|
//!
//! ## STT providers
//!
//! - `"cloud"` → the hosted transcription provider, when one is registered.
//! - `"<slug>:<model>"` → third-party STT API via the voice provider registry
//!   (e.g. `"deepgram:nova-2"`, `"openai:whisper-1"`, `"elevenlabs:scribe_v1"`).
//!
//! There is **no local STT branch**. The bundled whisper.cpp engine was removed;
//! which hosted engine runs is chosen by `voice_server.stt_engine` and resolved
//! by [`effective_stt_provider`].
//!
//! ## TTS providers
//!
//! - `"cloud"` → the hosted speech provider (with Oculus-15 visemes for the
//!   mascot lip-sync), when one is registered.
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
pub mod registry;
mod stt_providers;
mod traits;
mod tts_providers;

#[cfg(test)]
#[path = "factory_tests.rs"]
mod tests;

// Re-export the public API — exact visibility preserved from the original file.
pub use entry::{create_stt_provider, create_tts_provider, DEFAULT_PIPER_VOICE, DEFAULT_STT_MODEL};
pub use helpers::{effective_stt_provider, effective_tts_provider};
pub use stt_providers::ExternalSttProvider;
pub use traits::{SttProvider, SttResult, TtsProvider};
pub use tts_providers::{ExternalTtsProvider, PiperTtsProvider};
