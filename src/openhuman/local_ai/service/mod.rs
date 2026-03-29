//! Local Ollama / whisper / piper stack — implementation split across submodules.

mod assets;
mod bootstrap;
mod ollama_admin;
mod public_infer;
mod speech;
mod vision_embed;

use crate::openhuman::local_ai::types::LocalAiStatus;
use parking_lot::Mutex;

pub struct LocalAiService {
    pub(crate) status: Mutex<LocalAiStatus>,
    pub(crate) bootstrap_lock: tokio::sync::Mutex<()>,
    pub(crate) last_memory_summary_at: Mutex<Option<std::time::Instant>>,
    pub(crate) http: reqwest::Client,
}
