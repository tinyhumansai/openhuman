//! Voice domain — hosted speech-to-text and text-to-speech (local piper / hosted).
//!
//! Provides RPC endpoints under the `openhuman.voice_*` namespace for
//! transcription, synthesis, and proactive availability checking.
//!
//! Reusable inference mechanics live in `tinyinference-voice`; this module
//! binds them to OpenHuman configuration, authentication, providers, RPC,
//! and UI contracts.
//!
//! ## Compile-time gate (`voice` feature)
//!
//! `pub mod voice;` is ALWAYS compiled — it is a facade. The real
//! implementation (the submodules below) is gated behind the `voice` Cargo
//! feature. When the feature is off, [`stub`] takes its place and exposes the
//! same public surface that other-gated callers depend on (`create_stt_provider`
//! and `effective_stt_provider`) with no-op / disabled-error bodies. Keeping
//! the two surfaces in lockstep is enforced by the disabled-build check
//! (`cargo check --no-default-features --features "<all-but-voice>"`): any
//! signature drift fails that build.

// Ungated — part of the always-compiled facade (see the module docs above): it
// reports which side of the gate this binary landed on, so it must exist in
// both states.
pub mod compile_status;
pub use compile_status::VOICE_COMPILED_IN;

#[cfg(feature = "voice")]
pub mod factory;
#[cfg(feature = "voice")]
pub mod live;
#[cfg(feature = "voice")]
mod ops;
#[cfg(feature = "voice")]
pub mod realtime_harness;
pub mod reply_synthesis;
#[cfg(feature = "voice")]
mod schemas;
#[cfg(feature = "voice")]
mod types;

#[cfg(feature = "voice")]
pub mod local_speech;
#[cfg(feature = "voice")]
pub mod postprocess;
#[cfg(feature = "voice")]
pub use factory::{
    create_stt_provider, create_tts_provider, effective_stt_provider, effective_tts_provider,
    ExternalSttProvider, ExternalTtsProvider, SttProvider, SttResult, TtsProvider,
    DEFAULT_PIPER_VOICE, DEFAULT_STT_MODEL,
};
#[cfg(feature = "voice")]
pub use ops::*;
#[cfg(feature = "voice")]
pub(crate) use schemas::all_voice_controller_entries;
#[cfg(feature = "voice")]
pub use schemas::{all_voice_controller_schemas, all_voice_registered_controllers, voice_schemas};
#[cfg(feature = "voice")]
pub use types::{VoiceSpeechResult, VoiceStatus, VoiceTtsResult};

// ---------------------------------------------------------------------------
// Disabled facade — compiled only when the `voice` feature is OFF.
// ---------------------------------------------------------------------------

#[cfg(not(feature = "voice"))]
mod stub;
#[cfg(not(feature = "voice"))]
pub use stub::*;

#[cfg(all(test, feature = "voice"))]
pub(crate) use schemas::parameter_contracts;
