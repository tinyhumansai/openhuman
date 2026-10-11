//! Phase 2 — always-on listening.
//!
//! Instead of a hotkey gating each recording, always-on mode keeps the mic
//! open continuously and uses **voice-activity detection** to carve the audio
//! stream into utterances: an utterance opens when energy rises above an onset
//! threshold and closes after a configurable run of silence (the "hangover").
//! Each completed utterance is transcribed and pushed onto the dictation bus,
//! so it reaches the agent and the notch exactly like a hotkey dictation.
//!
//! ## Where the work happens
//!
//! Device I/O, callback queues and audio processing run in the loaded TinyVoice
//! module. The host polls bounded batches and owns configuration, permission,
//! transcription routing, privacy gates and lifecycle cleanup.
//!
//! Privacy: always-on is **opt-in** (`config.voice_server.always_on_enabled`,
//! default false) and pauses when the screen is locked.
//!
//! ## Module layout
//!
//! - [`capture`] — bounded bus forwarding and cancellation of the module stream.
//! - [`lock_watcher`] — the macOS screen-lock privacy hook.
//! - [`processor`] — the async pipeline: VAD session, segmentation, gating.
//! - [`transcribe`] — STT, wake-word gating, and local fast-path commands.

mod capture;
mod lock_watcher;
mod processor;
mod transcribe;

pub use processor::{shutdown, start_if_enabled, stop};

use processor::notch_status;
#[cfg(target_os = "macos")]
use processor::PAUSED;

#[cfg(test)]
use crate::modules::voice as tinyvoice;
#[cfg(test)]
use processor::ENABLED;

#[cfg(test)]
#[path = "always_on_tests.rs"]
mod tests;

const LOG_PREFIX: &str = "[voice::always_on]";
