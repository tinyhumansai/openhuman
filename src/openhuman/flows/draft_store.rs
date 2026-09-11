//! This host's binding of the authoring-draft store to its workspace.
//!
//! Same shape and same reason as [`super::store`]: the storage lives in
//! `tinyflows_sqlite::drafts` and takes a directory; this supplies the
//! directory. Drafts land in `<workspace_dir>/flows/drafts/<id>.json`.

use super::store::dir;
use crate::openhuman::config::Config;
use anyhow::Result;
use serde_json::Value;
use tinyflows_catalog::{DraftOrigin, FlowDraft};

/// Binds [`tinyflows_sqlite::drafts::create_draft`] to this host's catalog directory.
#[inline]
pub fn create_draft(
    config: &Config,
    flow_id: Option<String>,
    name: String,
    graph: Value,
    origin: DraftOrigin,
) -> Result<FlowDraft> {
    tinyflows_sqlite::drafts::create_draft(&dir(config), flow_id, name, graph, origin)
}

/// Binds [`tinyflows_sqlite::drafts::get_draft`] to this host's catalog directory.
#[inline]
pub fn get_draft(config: &Config, id: &str) -> Result<Option<FlowDraft>> {
    tinyflows_sqlite::drafts::get_draft(&dir(config), id)
}

/// Binds [`tinyflows_sqlite::drafts::update_draft`] to this host's catalog directory.
#[inline]
pub fn update_draft(
    config: &Config,
    id: &str,
    name: Option<String>,
    graph: Option<Value>,
    flow_id: Option<Option<String>>,
) -> Result<FlowDraft> {
    tinyflows_sqlite::drafts::update_draft(&dir(config), id, name, graph, flow_id)
}

/// Binds [`tinyflows_sqlite::drafts::list_drafts`] to this host's catalog directory.
#[inline]
pub fn list_drafts(config: &Config) -> Result<Vec<FlowDraft>> {
    tinyflows_sqlite::drafts::list_drafts(&dir(config))
}

/// Binds [`tinyflows_sqlite::drafts::delete_draft`] to this host's catalog directory.
#[inline]
pub fn delete_draft(config: &Config, id: &str) -> Result<bool> {
    tinyflows_sqlite::drafts::delete_draft(&dir(config), id)
}
