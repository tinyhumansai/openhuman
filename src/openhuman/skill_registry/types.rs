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
    /// Direct download URL for the SKILL.md file. Empty when the upstream
    /// source hosts the skill on a non-raw portal (ClawHub / LobeHub /
    /// skills.sh) with no fetchable `SKILL.md`; install surfaces an actionable
    /// error pointing at [`source_url`] instead of a misleading 404.
    pub download_url: String,
    /// Human-facing source page for the skill (GitHub blob/tree, LobeHub,
    /// ClawHub, skills.sh, …). Carried from the catalog's `sourceUrl`; used to
    /// derive the raw download URL for GitHub-hosted community skills and to
    /// give the user a link when no direct download exists. See issue #3741.
    pub source_url: Option<String>,
    /// Docs path from the Hermes catalog.
    pub docs_path: Option<String>,
    /// Required CLI commands.
    pub commands: Vec<String>,
    /// Required environment variables.
    pub env_vars: Vec<String>,
    /// Software license.
    pub license: Option<String>,
}
