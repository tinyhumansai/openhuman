//! Host clients for TinyFlows Catalog draft operations.

use anyhow::Result;
use serde_json::Value;
use tinyflows_bus::catalog::{
    self, CreateDraftRequest, DraftIdRequest, DraftOrigin, FlowDraft, UpdateDraftRequest,
};

use crate::config::Config;
use crate::flows::{catalog_storage, store};

/// Create a module-owned editable workflow draft.
pub fn create_draft(
    config: &Config,
    flow_id: Option<String>,
    name: String,
    graph: Value,
    origin: DraftOrigin,
) -> Result<FlowDraft> {
    let storage = catalog_storage::register_blocking(config)?;
    let request = CreateDraftRequest {
        storage,
        flow_id,
        name,
        graph,
        origin,
    };
    store::call_catalog::<FlowDraft, _>(config, catalog::members::CREATE_DRAFT, (request,))
}

/// Read one module-owned draft.
pub fn get_draft(config: &Config, id: &str) -> Result<Option<FlowDraft>> {
    let storage = catalog_storage::register_blocking(config)?;
    let request = DraftIdRequest {
        storage,
        id: id.to_owned(),
    };
    let draft: Option<FlowDraft> =
        store::call_catalog(config, catalog::members::GET_DRAFT, (request,))?;
    Ok(draft)
}

/// Patch a module-owned draft.
pub fn update_draft(
    config: &Config,
    id: &str,
    name: Option<String>,
    graph: Option<Value>,
    flow_id: Option<Option<String>>,
) -> Result<FlowDraft> {
    let storage = catalog_storage::register_blocking(config)?;
    let request = UpdateDraftRequest {
        storage,
        id: id.to_owned(),
        name,
        graph,
        flow_id,
    };
    store::call_catalog(config, catalog::members::UPDATE_DRAFT, (request,))
}

/// List module-owned drafts.
pub fn list_drafts(config: &Config) -> Result<Vec<FlowDraft>> {
    let storage = catalog_storage::register_blocking(config)?;
    let drafts: Vec<FlowDraft> =
        store::call_catalog(config, catalog::members::LIST_DRAFTS, (storage,))?;
    Ok(drafts)
}

/// Delete one draft, returning whether a record was removed.
pub fn delete_draft(config: &Config, id: &str) -> Result<bool> {
    let storage = catalog_storage::register_blocking(config)?;
    let request = DraftIdRequest {
        storage,
        id: id.to_owned(),
    };
    store::call_catalog(config, catalog::members::DELETE_DRAFT, (request,))
}
