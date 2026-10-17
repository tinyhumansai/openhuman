//! This host's binding of the authoring-draft store to its workspace.
//!
//! Same shape and same reason as [`super::store`]: drafts are documents in the
//! flow catalog (`FlowCatalogDocuments`), in the acting scope of the storage
//! backend. Drafts the retired JSON store kept in
//! `<workspace_dir>/flows/drafts/<id>.json` are imported on first use.

use super::store::{documents, run};
use crate::config::Config;
use anyhow::Result;
use serde_json::Value;
use tinyflows_catalog::{DraftOrigin, FlowDraft};

/// The catalog `create_draft` for this call.
#[inline]
pub fn create_draft(
    config: &Config,
    flow_id: Option<String>,
    name: String,
    graph: Value,
    origin: DraftOrigin,
) -> Result<FlowDraft> {
    let docs = documents(config)?;
    run(async move { docs.create_draft(flow_id, name, graph, origin).await })
}

/// The catalog `get_draft` for this call.
#[inline]
pub fn get_draft(config: &Config, id: &str) -> Result<Option<FlowDraft>> {
    let docs = documents(config)?;
    let id = id.to_string();
    run(async move { docs.get_draft(&id).await })
}

/// The catalog `update_draft` for this call.
#[inline]
pub fn update_draft(
    config: &Config,
    id: &str,
    name: Option<String>,
    graph: Option<Value>,
    flow_id: Option<Option<String>>,
) -> Result<FlowDraft> {
    let docs = documents(config)?;
    let id = id.to_string();
    run(async move { docs.update_draft(&id, name, graph, flow_id).await })
}

/// The catalog `list_drafts` for this call.
#[inline]
pub fn list_drafts(config: &Config) -> Result<Vec<FlowDraft>> {
    let docs = documents(config)?;
    run(async move { docs.list_drafts().await })
}

/// The catalog `delete_draft` for this call.
#[inline]
pub fn delete_draft(config: &Config, id: &str) -> Result<bool> {
    let docs = documents(config)?;
    let id = id.to_string();
    run(async move { docs.delete_draft(&id).await })
}
