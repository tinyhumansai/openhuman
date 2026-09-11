//! Model routing, embedding routing, and query classification.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ModelRouteConfig {
    pub hint: String,
    pub model: String,
}

/// A per-workload embedding provider override. Defined in the contract crate —
/// the memory store's embedder factory reads it. See
/// [`tinymemory_api::host::EmbeddingRouteConfig`].
pub use tinymemory_api::host::EmbeddingRouteConfig;
