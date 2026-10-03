//! Model routing, embedding routing, and query classification.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ModelRouteConfig {
    pub hint: String,
    pub model: String,
}

/// A per-workload embedding provider override (`[[embedding_routes]]`).
/// Inert serde data; its serde form is persisted in users' `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EmbeddingRouteConfig {
    pub hint: String,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub dimensions: Option<usize>,
}
