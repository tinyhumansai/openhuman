//! Composio sources: connector records become `Document` items.
//!
//! A Composio source names a toolkit (`gmail`, `notion`, …). Syncing it runs
//! the connector module's sync for each active connection of that toolkit
//! (`integrations::composio::ops::run_sync_pass`), which pages the account and
//! hands back decoded records; each record is stored as a document with
//! `source = composio:<source id>`, `tags = [toolkit, "connection:<id>", …]`,
//! its URL and its upstream timestamp. The connection tag is what deleting a
//! connection with `clear_memory` forgets by. The same conversion backs `openhuman.composio_sync`,
//! which syncs one connection on demand.

use chrono::{TimeZone, Utc};
use tinyconnectors_bus::records::ConnectorRecord;
use tinymemory::{DocumentBody, MemoryMeta, SourceKind, SourceRef, StoreItem};

use crate::config::schema::MemorySourceConfig;
use crate::config::Config;
use crate::integrations::composio::ops::{
    active_connection_ids, run_sync_pass, SYNC_PASS_MAX_ITEMS,
};
use crate::memory::engine::BoundEngine;
use crate::memory::error::{MemoryError, MemoryResult};

/// Most connector passes one source sync runs per connection.
const MAX_PASSES_PER_CONNECTION: usize = 25;

/// The tag every item synced through a connection carries.
#[must_use]
pub fn connection_tag(connection_id: &str) -> String {
    format!("connection:{connection_id}")
}

/// The document one connector record stores, or `None` for an empty record.
#[must_use]
pub fn record_item(
    toolkit: &str,
    connection_id: &str,
    source_id: &str,
    record: &ConnectorRecord,
) -> Option<StoreItem> {
    if record.content.trim().is_empty() {
        return None;
    }
    let mut tags = vec![toolkit.to_ascii_lowercase(), connection_tag(connection_id)];
    for tag in &record.tags {
        if !tag.trim().is_empty() && !tags.contains(tag) {
            tags.push(tag.clone());
        }
    }
    let observed_at = record
        .updated_at_ms
        .and_then(|ms| Utc.timestamp_millis_opt(ms).single());
    Some(StoreItem::Document {
        title: Some(record.title.trim().to_string()).filter(|title| !title.is_empty()),
        body: DocumentBody::Text(record.content.clone()),
        mime: record.mime.clone(),
        meta: MemoryMeta {
            url: record.url.clone(),
            tags,
            observed_at,
            source: SourceRef {
                kind: SourceKind::Composio,
                id: Some(source_id.to_string()),
            },
            ..MemoryMeta::default()
        },
    })
}

/// Stores every non-empty record. Returns how many were stored.
pub async fn store_records(
    bound: &BoundEngine,
    toolkit: &str,
    connection_id: &str,
    source_id: &str,
    records: &[ConnectorRecord],
) -> MemoryResult<u64> {
    let items: Vec<StoreItem> = records
        .iter()
        .filter_map(|record| record_item(toolkit, connection_id, source_id, record))
        .collect();
    super::sync::store_all(bound, items, source_id).await
}

/// Syncs every active connection of the source's toolkit.
pub async fn sync_toolkit(
    config: &Config,
    bound: &BoundEngine,
    source: &MemorySourceConfig,
) -> MemoryResult<u64> {
    let toolkit = source.target.as_str();
    let connections = active_connection_ids(config, toolkit)
        .await
        .map_err(MemoryError::Engine)?;
    if connections.is_empty() {
        return Err(MemoryError::invalid(format!(
            "no active {toolkit} connection; connect it first"
        )));
    }
    let mut stored = 0u64;
    for connection_id in connections {
        for _ in 0..MAX_PASSES_PER_CONNECTION {
            let pass = run_sync_pass(
                config,
                bound,
                toolkit,
                &connection_id,
                &source.id,
                "manual",
                SYNC_PASS_MAX_ITEMS,
            )
            .await
            .map_err(MemoryError::Engine)?;
            stored += pass.written;
            if let Some(failure) = pass.failure {
                return Err(MemoryError::Engine(failure));
            }
            if !pass.more_pending {
                break;
            }
        }
    }
    Ok(stored)
}

/// The memory source id a Composio sync of `toolkit` files its items under:
/// the configured `composio` source for the toolkit when there is one, else
/// `composio:<toolkit>`.
#[must_use]
pub fn source_id_for_toolkit(config: &Config, toolkit: &str) -> String {
    let toolkit = toolkit.to_ascii_lowercase();
    config
        .memory
        .sources
        .iter()
        .find(|source| {
            source.kind == crate::config::schema::MemorySourceKind::Composio
                && source.target == toolkit
        })
        .map_or_else(|| format!("composio:{toolkit}"), |source| source.id.clone())
}

/// Forgets every item synced through `connection_id`. Memory off forgets
/// nothing and is not an error.
pub async fn forget_connection(config: &Config, connection_id: &str) -> MemoryResult<usize> {
    let bound = match crate::memory::engine::resolve(config).engine() {
        Ok(bound) => bound,
        Err(MemoryError::Off(_)) => return Ok(0),
        Err(error) => return Err(error),
    };
    let filter = tinymemory::MetaFilter {
        sources: vec![SourceKind::Composio],
        tags_any: vec![connection_tag(connection_id)],
        ..tinymemory::MetaFilter::default()
    };
    let report = bound
        .engine
        .forget(tinymemory::ForgetTarget::Filter(filter))
        .await?;
    Ok(report.forgotten)
}

#[cfg(test)]
#[path = "composio_tests.rs"]
mod tests;
