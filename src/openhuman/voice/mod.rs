//! Voice domain — speech-to-text (whisper.cpp) and text-to-speech (piper).
//!
//! Provides RPC endpoints under the `openhuman.voice_*` namespace for
//! transcription, synthesis, and proactive availability checking.

mod ops;
mod postprocess;
mod schemas;
mod types;

pub use ops::*;
pub use schemas::{all_voice_controller_schemas, all_voice_registered_controllers, voice_schemas};
pub use types::{VoiceSpeechResult, VoiceStatus, VoiceTtsResult};
