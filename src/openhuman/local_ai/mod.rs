//! Bundled local AI stack (Ollama, whisper.cpp, Piper).

pub mod rpc;
mod schemas;

mod install;
mod model_ids;
mod ollama_api;
mod parse;
mod paths;
mod service;
mod types;

pub use schemas::{
    all_controller_schemas as all_local_ai_controller_schemas,
    all_registered_controllers as all_local_ai_registered_controllers,
};
pub use service::LocalAiService;
pub use types::{
    LocalAiAssetStatus, LocalAiAssetsStatus, LocalAiDownloadProgressItem, LocalAiDownloadsProgress,
    LocalAiEmbeddingResult, LocalAiSpeechResult, LocalAiStatus, LocalAiTtsResult, Suggestion,
};

use std::path::PathBuf;
use std::sync::Arc;

use crate::openhuman::config::Config;

use model_ids::effective_chat_model_id;

static LOCAL_AI: once_cell::sync::OnceCell<Arc<LocalAiService>> = once_cell::sync::OnceCell::new();

pub fn global(config: &Config) -> Arc<LocalAiService> {
    LOCAL_AI
        .get_or_init(|| Arc::new(LocalAiService::new(config)))
        .clone()
}

pub fn model_artifact_path(config: &Config) -> PathBuf {
    let root = config
        .config_path
        .parent()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| config.workspace_dir.clone());
    root.join("models")
        .join("local-ai")
        .join(effective_chat_model_id(config).replace(':', "-") + ".ollama")
}
