//! Shared test fixtures for saved workflows.

use anyhow::{anyhow, Result};
use serde_json::Value;
use tinyflows_bus::catalog::Workflow as Flow;
use tinyflows_bus::catalog::{
    self, CatalogDocumentPrecondition, CatalogStorageReply, ListWorkflowsResult,
};

use crate::{config::Config, flows::store};

/// Seed a caller-chosen workflow id through the existing CatalogStorage port.
pub(crate) fn seed_flow(config: &Config, flow: &Flow) -> Result<()> {
    let storage = super::catalog_storage::register_blocking(config)?;
    let _: ListWorkflowsResult =
        store::call_catalog(config, catalog::members::LIST_WORKFLOWS, (storage.clone(),))?;
    let created_ms = chrono::DateTime::parse_from_rfc3339(&flow.created_at)
        .map(|date| date.timestamp_millis())
        .unwrap_or(0);
    let updated_ms = chrono::DateTime::parse_from_rfc3339(&flow.updated_at)
        .map(|date| date.timestamp_millis())
        .unwrap_or(created_ms);
    let mut doc = serde_json::json!({
        "name": flow.name,
        "graph": flow.graph,
        "enabled": flow.enabled,
        "created_ms": created_ms,
        "updated_ms": updated_ms,
        "require_approval": flow.require_approval,
        "revision": 1,
        "incarnation": uuid::Uuid::new_v4().to_string(),
    });
    if let Some(status) = &flow.last_status {
        doc["last_status"] = Value::String(status.clone());
    }
    if let Some(last_run_at) = &flow.last_run_at {
        if let Ok(date) = chrono::DateTime::parse_from_rfc3339(last_run_at) {
            doc["last_run_ms"] = Value::from(date.timestamp_millis());
        }
    }
    let request = catalog::DocPutRequest {
        collection: "flows.definitions".into(),
        id: flow.id.clone(),
        doc,
        precondition: CatalogDocumentPrecondition::None,
        schema: 2,
        expires_ms: None,
        created_ms: Some(created_ms),
    };
    match super::catalog_storage::call_test_storage::<CatalogStorageReply, _>(
        config,
        catalog::storage::PUT,
        (request,),
    )? {
        CatalogStorageReply::Value { .. } => Ok(()),
        CatalogStorageReply::Error { error } => Err(anyhow!(
            "CatalogStorage fixture write failed ({}): {}",
            error.kind,
            error.message
        )),
    }
}
