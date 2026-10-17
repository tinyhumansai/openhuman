//! Standalone voice server — hotkey → record → transcribe → insert text.
//!
//! Can run as part of the core process or independently via the CLI.
//! The server listens for a configurable hotkey, records audio from the
//! microphone, transcribes via the configured STT engine, and inserts the result into the
//! active text field.
//!
//! ## Module layout
//!
//! - [`types`] — run state, status snapshot, and server config.
//! - [`hotkey_listener`] — TinyVoice native listeners and the macOS Globe composition.
//! - [`pipeline`] — the background recording → transcription pass.
//! - [`runtime`] — the hotkey event loop that drives a `VoiceServer`.
//! - [`singleton`] — the process-global instance, auto-start, and the
//!   standalone (CLI) entry point.

pub(crate) mod hotkey_listener;
mod pipeline;
mod recorder;
mod runtime;
mod singleton;
mod types;

pub use runtime::VoiceServer;
pub use singleton::{global_server, run_standalone, start_if_enabled, try_global_server};
pub use types::{ServerState, VoiceServerConfig, VoiceServerStatus};

#[cfg(test)]
use crate::config::Config;
#[cfg(test)]
use crate::voice::audio_capture::RecordingHandle;
#[cfg(test)]
use pipeline::{
    build_initial_prompt, capture_expected_app_name, process_recording_bg, push_recent_transcript,
    truncate_for_log,
};
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use tinyvoice_bus::hotkey::ActivationMode;
#[cfg(test)]
use tokio::sync::Mutex;
#[cfg(test)]
use types::DEFAULT_SILENCE_THRESHOLD;

const LOG_PREFIX: &str = "[voice_server]";

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
