//! Local Ollama / whisper / piper stack — implementation split across submodules.

mod assets;
mod bootstrap;
mod lm_studio;
pub(crate) mod ollama_admin;
mod public_infer;
pub(crate) mod spawn_marker;
mod speech;
mod vision_embed;
pub(crate) mod whisper_engine;

use crate::openhuman::inference::types::LocalAiStatus;
use parking_lot::Mutex;

pub struct LocalAiService {
    pub(crate) status: Mutex<LocalAiStatus>,
    pub(crate) bootstrap_lock: tokio::sync::Mutex<()>,
    pub(crate) whisper_load_lock: tokio::sync::Mutex<()>,
    pub(crate) last_memory_summary_at: Mutex<Option<std::time::Instant>>,
    pub(crate) http: reqwest::Client,
    /// In-process whisper.cpp context for low-latency STT.
    pub(crate) whisper: whisper_engine::WhisperEngineHandle,
    /// Handle to any `ollama serve` openhuman itself spawned. `None` when
    /// the daemon currently on `:11434` was started outside openhuman (and
    /// adopted via the health probe) — those are never killed on exit.
    pub(crate) owned_ollama: Mutex<Option<tokio::process::Child>>,
}

impl LocalAiService {
    /// Returns `true` iff openhuman currently holds an owned Ollama child handle.
    ///
    /// Intended for tests and health-check callers that need to inspect the
    /// ownership state without going through the full bootstrap path.
    pub fn has_owned_ollama(&self) -> bool {
        self.owned_ollama.lock().is_some()
    }

    /// Inject a pre-spawned child as the owned Ollama handle.
    ///
    /// This allows integration tests to set up the ownership state without
    /// running the full `start_and_wait_for_server` path (which requires a
    /// real Ollama binary). Production code uses the internal field directly
    /// inside `ollama_admin.rs`; this method is the public bridge for the
    /// `tests/` integration test crate.
    pub fn inject_owned_ollama(&self, child: tokio::process::Child) {
        *self.owned_ollama.lock() = Some(child);
    }
}
