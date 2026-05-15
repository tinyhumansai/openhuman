//! Local Ollama / whisper / piper stack — implementation split across submodules.

mod assets;
mod bootstrap;
mod lm_studio;
mod ollama_admin;
mod public_infer;
pub(crate) mod spawn_marker;
mod speech;
mod vision_embed;
pub(crate) mod whisper_engine;

use crate::openhuman::local_ai::types::LocalAiStatus;
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
