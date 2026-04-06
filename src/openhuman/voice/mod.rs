//! Voice domain — speech-to-text (whisper.cpp) and text-to-speech (piper).
//!
//! Provides RPC endpoints under the `openhuman.voice_*` namespace for
//! transcription, synthesis, proactive availability checking, and a
//! standalone voice dictation server (hotkey → record → transcribe → insert).

pub mod audio_capture;
pub mod hotkey;
mod ops;
mod postprocess;
mod schemas;
pub mod server;
pub mod streaming;
pub mod text_input;
mod types;

pub use ops::*;
pub use schemas::{all_voice_controller_schemas, all_voice_registered_controllers, voice_schemas};
pub use types::{VoiceSpeechResult, VoiceStatus, VoiceTtsResult};
