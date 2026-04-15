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
mod tests {
    use super::*;

    #[test]
    fn model_artifact_path_includes_models_local_ai_subdirs() {
        let config = Config::default();
        let path = model_artifact_path(&config);
        let path_str = path.to_string_lossy();
        assert!(
            path_str.contains("models"),
            "expected `models` in path: {path_str}"
        );
        assert!(
            path_str.contains("local-ai"),
            "expected `local-ai` subdir in path: {path_str}"
        );
    }

    #[test]
    fn model_artifact_path_ends_with_ollama_suffix() {
        let config = Config::default();
        let path = model_artifact_path(&config);
        assert_eq!(
            path.extension().and_then(|s| s.to_str()),
            Some("ollama"),
            "model artifact must have `.ollama` extension: {}",
            path.display()
        );
    }

    #[test]
    fn model_artifact_path_replaces_colon_in_model_id_with_dash() {
        // Model IDs commonly look like `qwen2:1.5b`; colons are illegal on
        // Windows path components, so we normalise to `-`. This test pins
        // that mapping.
        let config = Config::default();
        let path = model_artifact_path(&config);
        let file = path.file_name().unwrap().to_string_lossy().to_string();
        assert!(!file.contains(':'), "filename must not contain `:`: {file}");
    }

    #[test]
    fn global_returns_same_arc_across_calls() {
        let config = Config::default();
        let a = global(&config);
        let b = global(&config);
        assert!(Arc::ptr_eq(&a, &b), "global() must return a shared Arc");
    }
}
