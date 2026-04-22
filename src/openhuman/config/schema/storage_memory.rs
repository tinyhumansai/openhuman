//! Storage provider and memory configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct StorageConfig {
    #[serde(default)]
    pub provider: StorageProviderSection,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct StorageProviderSection {
    #[serde(default)]
    pub config: StorageProviderConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StorageProviderConfig {
    #[serde(default)]
    pub provider: String,
}

impl Default for StorageProviderConfig {
    fn default() -> Self {
        Self {
            provider: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[allow(clippy::struct_excessive_bools)]
pub struct MemoryConfig {
    pub backend: String,
    pub auto_save: bool,
    #[serde(default = "default_embedding_provider")]
    pub embedding_provider: String,
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,
    #[serde(default = "default_embedding_dims")]
    pub embedding_dimensions: usize,
    #[serde(default = "default_min_relevance_score")]
    pub min_relevance_score: f64,
    #[serde(default)]
    pub response_cache_enabled: bool,
    #[serde(default)]
    pub sqlite_open_timeout_secs: Option<u64>,
}

fn default_embedding_provider() -> String {
    "ollama".into()
}
fn default_embedding_model() -> String {
    "nomic-embed-text:latest".into()
}
fn default_embedding_dims() -> usize {
    768
}
fn default_min_relevance_score() -> f64 {
    0.4
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            backend: "sqlite".into(),
            auto_save: true,
            embedding_provider: default_embedding_provider(),
            embedding_model: default_embedding_model(),
            embedding_dimensions: default_embedding_dims(),
            min_relevance_score: default_min_relevance_score(),
            response_cache_enabled: false,
            sqlite_open_timeout_secs: None,
        }
    }
}
