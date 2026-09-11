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

/// Like [`global`] but returns `None` instead of initialising the singleton.
///
/// Useful from shutdown paths where lazy-creating the service just to call a
/// no-op cleanup would be wasteful — if local AI was never used in this
/// process, there's nothing to clean up.
pub fn try_global() -> Option<Arc<LocalAiService>> {
    LOCAL_AI.get().cloned()
}

pub fn model_artifact_path(config: &Config) -> PathBuf {
    let root = crate::openhuman::config::default_root_openhuman_dir().unwrap_or_else(|_| {
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

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
