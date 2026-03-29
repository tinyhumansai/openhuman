//! Public input/output types for namespace memory documents.

use serde::{Deserialize, Serialize};

pub(crate) const GLOBAL_NAMESPACE: &str = "global";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceDocumentInput {
    pub namespace: String,
    pub key: String,
    pub title: String,
    pub content: String,
    pub source_type: String,
    pub priority: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub category: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub document_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceQueryResult {
    pub key: String,
    pub content: String,
    pub score: f64,
}
