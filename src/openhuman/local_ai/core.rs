use std::path::PathBuf;
use std::sync::Arc;

use crate::openhuman::config::Config;

use super::model_ids::effective_chat_model_id;
use super::service::LocalAiService;

static LOCAL_AI: once_cell::sync::OnceCell<Arc<LocalAiService>> = once_cell::sync::OnceCell::new();

pub fn global(config: &Config) -> Arc<LocalAiService> {
    LOCAL_AI
        .get_or_init(|| Arc::new(LocalAiService::new(config)))
        .clone()
}

pub fn model_artifact_path(config: &Config) -> PathBuf {
    let root = crate::openhuman::config::default_root_openhuman_dir()
        .unwrap_or_else(|_| {
            config
                .config_path
                .parent()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| config.workspace_dir.clone())
        });
    root.join("models")
        .join("local-ai")
        .join(effective_chat_model_id(config).replace(':', "-") + ".ollama")
}
