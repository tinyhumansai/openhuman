//! Wire-format types for `openhuman.skill_registry_*` RPC methods.

use serde::{Deserialize, Serialize};

use crate::core::ControllerSchema;
use crate::openhuman::skill_registry::types::CatalogEntry;
use crate::openhuman::workflows::ops_types::WorkflowScope;

// ── Params ──────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub(super) struct BrowseParams {
    #[serde(default)]
    pub(super) force_refresh: bool,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct SearchParams {
    #[serde(default)]
    pub(super) query: String,
    #[serde(default)]
    pub(super) source: Option<String>,
    #[serde(default)]
    pub(super) category: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct InstallParams {
    pub(super) entry_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct UninstallParams {
    pub(super) name: String,
}

// ── Results ─────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub(super) struct BrowseResult {
    pub(super) entries: Vec<CatalogEntry>,
}

#[derive(Debug, Serialize)]
pub(super) struct SearchResult {
    pub(super) entries: Vec<CatalogEntry>,
}

#[derive(Debug, Serialize)]
pub(super) struct SourcesResult {
    pub(super) sources: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct CategoriesResult {
    pub(super) categories: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct InstallResult {
    pub(super) url: String,
    pub(super) stdout: String,
    pub(super) stderr: String,
    pub(super) new_skills: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct UninstallResult {
    pub(super) name: String,
    pub(super) removed_path: String,
    pub(super) scope: WorkflowScope,
}

#[derive(Debug, Serialize)]
pub(super) struct SchemasResult {
    pub(super) schemas: Vec<ControllerSchema>,
}
