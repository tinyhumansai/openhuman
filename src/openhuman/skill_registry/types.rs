//! Domain types for the skill registry.

use serde::{Deserialize, Serialize};

/// One entry in the indexed skill catalog.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogEntry {
    /// Unique slug (e.g. "apple-notes", "docker-manager").
    pub id: String,
    /// Display name.
    pub name: String,
    /// Short description.
    pub description: String,
    /// Upstream source within the aggregated catalog (e.g. "built-in",
    /// "optional", "ClawHub", "skills.sh", "LobeHub", "browse.sh").
    pub source: String,
    /// Category label from the upstream catalog.
    pub category: String,
    /// Author name, if known.
    pub author: Option<String>,
    /// Version string, if declared.
    pub version: Option<String>,
    /// Tags for search/filter.
    pub tags: Vec<String>,
    /// Compatible platform hints.
    pub platforms: Vec<String>,
    /// Direct download URL for the SKILL.md file.
    pub download_url: String,
    /// Docs path from the Hermes catalog.
    pub docs_path: Option<String>,
    /// Required CLI commands.
    pub commands: Vec<String>,
    /// Required environment variables.
    pub env_vars: Vec<String>,
    /// Software license.
    pub license: Option<String>,
}
