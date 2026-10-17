//! The `task_sources` owner's storage schema: the configured sources and the
//! ingestion ledger, with the steps that bring the v1 collections forward.
//!
//! | v1 | v2 | change |
//! |---|---|---|
//! | `task_sources` | `task_sources.sources` | `created_at` dropped (`created_ms` kept), `last_fetch_at` becomes `last_fetch_ms` |
//! | `ingested_tasks` (id `<len>:<source>/<external>`) | `task_sources.ingested` (id `Key::of([source_id, external_id])`) | `ingested_at` and `ingested_ns` dropped |
//!
//! `filter`, `target` and the task `payload` stay JSON text: they carry
//! provider-defined documents whose keys a document-store driver (MongoDB)
//! must never see as field names.

use serde_json::Value;
use tinystoragedrivers::schema::{Schema, ScopeClass, SourceRecord, Step, TargetRecord};
use tinystoragedrivers::{CollectionSpec, IndexSpec, Key, Result};

use crate::storage::schema_support::{key_of, owner, time_ms, Body};

pub(super) const SOURCES: &str = "task_sources.sources";
pub(super) const INGESTED: &str = "task_sources.ingested";
pub(super) const LEGACY_SOURCES: &str = "task_sources";
pub(super) const LEGACY_INGESTED: &str = "ingested_tasks";
const SCHEMA: u32 = 2;

/// What the task-source store keeps and the steps from the v1 collections.
#[must_use]
pub fn storage_schema() -> Schema {
    let [sources, ingested] = specs();
    Schema::new(owner("task_sources"))
        .collection_with(SCHEMA, sources, ScopeClass::Tenant)
        .collection_with(SCHEMA, ingested, ScopeClass::Tenant)
        .step(Step::new("sources_v2", SOURCES, 1, SCHEMA, sources_v2).source(LEGACY_SOURCES))
        .step(Step::new("ingested_v2", INGESTED, 1, SCHEMA, ingested_v2).source(LEGACY_INGESTED))
}

/// The two v2 collections with their indexes.
pub(super) fn specs() -> [CollectionSpec; 2] {
    [
        CollectionSpec::new(SOURCES).index(IndexSpec::new("by_created", ["created_ms"])),
        CollectionSpec::new(INGESTED)
            .index(IndexSpec::new("by_source", ["source_id", "ingested_ms"])),
    ]
}

/// The v1 collections as the legacy importer declares them.
pub(super) fn legacy_collections() -> Vec<CollectionSpec> {
    vec![
        CollectionSpec::new(LEGACY_SOURCES).index(IndexSpec::new("by_created", ["created_ms"])),
        CollectionSpec::new(LEGACY_INGESTED)
            .index(IndexSpec::new("by_source", ["source_id", "ingested_ms"])),
    ]
}

/// The ledger id for one `(source, external task)`.
///
/// # Panics
///
/// Never in practice: a two-part key is valid.
pub(super) fn ingested_id(source_id: &str, external_id: &str) -> String {
    #[allow(clippy::expect_used)]
    Key::of([source_id, external_id])
        .expect("a two-part key is valid")
        .as_str()
        .to_owned()
}

fn sources_v2(batch: Vec<SourceRecord>) -> Result<Vec<TargetRecord>> {
    batch
        .into_iter()
        .map(|record| {
            let old = &record.body;
            let created_ms = old
                .get("created_ms")
                .and_then(Value::as_i64)
                .or_else(|| time_ms(old, "created_at"))
                .unwrap_or(record.meta.created_ms);
            let last_fetch_ms = old
                .get("last_fetch_ms")
                .and_then(Value::as_i64)
                .or_else(|| time_ms(old, "last_fetch_at"));
            let body = Body::new()
                .copy(old, "provider")
                .copy(old, "enabled")
                .copy(old, "filter")
                .copy(old, "interval_secs")
                .copy(old, "target")
                .copy(old, "max_tasks_per_fetch")
                .copy(old, "connection_id")
                .copy(old, "name")
                .copy(old, "last_status")
                .set("created_ms", created_ms)
                .set_some("last_fetch_ms", last_fetch_ms)
                .build();
            Ok(TargetRecord::new(SOURCES, key_of(&record.id)?, body).created_ms(created_ms))
        })
        .collect()
}

fn ingested_v2(batch: Vec<SourceRecord>) -> Result<Vec<TargetRecord>> {
    Ok(batch
        .into_iter()
        .filter_map(|record| {
            let old = &record.body;
            let source_id = old.get("source_id").and_then(Value::as_str)?;
            let external_id = old.get("external_id").and_then(Value::as_str)?;
            let ingested_ms = old
                .get("ingested_ms")
                .and_then(Value::as_i64)
                .or_else(|| time_ms(old, "ingested_at"))
                .unwrap_or(record.meta.created_ms);
            let body = Body::new()
                .set("source_id", source_id)
                .set("external_id", external_id)
                .copy(old, "content_hash")
                .copy(old, "title")
                .copy(old, "payload")
                .set("ingested_ms", ingested_ms)
                .build();
            Some(
                TargetRecord::new(INGESTED, ingested_id(source_id, external_id), body)
                    .created_ms(ingested_ms),
            )
        })
        .collect())
}

#[cfg(test)]
#[path = "storage_schema_tests.rs"]
mod tests;
