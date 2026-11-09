//! The `notifications` owner's storage schema: integration notifications, the
//! content dedup window, per-provider settings and persisted core events, with
//! the steps that bring the v1 collections forward.
//!
//! | v1 | v2 | change |
//! |---|---|---|
//! | `integration_notifications` | `notifications.integration` | `received_at` dropped (`received_ms` kept), `scored_at` becomes `scored_ms` |
//! | `notification_dedup` | `notifications.dedup` | rename |
//! | `notification_settings` | `notifications.settings` | rename |
//! | `core_notifications` (id `<len>:<workspace>/<event>`) | `notifications.core` (id `Key::of([workspace, event])`) | `created_at` moves to metadata |
//!
//! `raw_payload` and the core event `payload` stay JSON text: they are
//! provider-controlled documents whose keys a document-store driver must never
//! see as field names.

use anyhow::Result;
use serde_json::Value;
use tinystoragedrivers::{CollectionSpec, IndexSpec};

use crate::storage::schema_support::{
    key_of, key_of_parts, map_legacy_collection, time_ms, Body, LegacySourceRecord as SourceRecord,
    LegacyTargetRecord as TargetRecord,
};

pub(super) const NOTIFICATIONS: &str = "notifications.integration";
pub(super) const DEDUP: &str = "notifications.dedup";
pub(super) const SETTINGS: &str = "notifications.settings";
pub(super) const CORE: &str = "notifications.core";

pub(super) const LEGACY_NOTIFICATIONS: &str = "integration_notifications";
pub(super) const LEGACY_DEDUP: &str = "notification_dedup";
pub(super) const LEGACY_SETTINGS: &str = "notification_settings";
pub(super) const LEGACY_CORE: &str = "core_notifications";

/// Converts legacy notification documents into the current owner shapes.
pub(crate) fn upgrade_records(
    records: Vec<crate::storage::import::Record>,
) -> Result<Vec<crate::storage::import::Record>> {
    let records = map_legacy_collection(records, LEGACY_NOTIFICATIONS, integration_v2)?;
    let records = map_legacy_collection(records, LEGACY_DEDUP, renamed(DEDUP))?;
    let records = map_legacy_collection(records, LEGACY_SETTINGS, renamed(SETTINGS))?;
    map_legacy_collection(records, LEGACY_CORE, core_v2)
}

/// The four v2 collections with their indexes.
pub(super) fn specs() -> Vec<CollectionSpec> {
    vec![
        CollectionSpec::new(NOTIFICATIONS)
            .index(IndexSpec::new("by_provider", ["provider", "received_ms"]))
            .index(IndexSpec::new("by_status", ["status"])),
        CollectionSpec::new(DEDUP),
        CollectionSpec::new(SETTINGS),
        CollectionSpec::new(CORE).index(IndexSpec::new(
            "by_read",
            ["workspace", "read", "timestamp_ms"],
        )),
    ]
}

/// Collections owned by the notification store.
pub(crate) fn collection_specs() -> Vec<CollectionSpec> {
    specs()
}

/// The v1 collections as the legacy importer declares them.
pub(super) fn legacy_collections() -> Vec<CollectionSpec> {
    vec![
        CollectionSpec::new(LEGACY_NOTIFICATIONS)
            .index(IndexSpec::new("by_provider", ["provider", "received_ms"]))
            .index(IndexSpec::new("by_status", ["status"])),
        CollectionSpec::new(LEGACY_DEDUP),
        CollectionSpec::new(LEGACY_SETTINGS),
        CollectionSpec::new(LEGACY_CORE).index(IndexSpec::new(
            "by_read",
            ["workspace", "read", "timestamp_ms"],
        )),
    ]
}

/// The core-notification document id.
///
/// # Panics
///
/// Never in practice: a two-part key is valid.
pub(super) fn core_id(workspace: &str, event_id: &str) -> String {
    #[allow(clippy::expect_used)]
    key_of_parts([workspace, event_id]).expect("a two-part key is valid")
}

fn integration_v2(batch: Vec<SourceRecord>) -> Result<Vec<TargetRecord>> {
    batch
        .into_iter()
        .map(|record| {
            let old = &record.body;
            let received_ms = old
                .get("received_ms")
                .and_then(Value::as_i64)
                .or_else(|| time_ms(old, "received_at"))
                .unwrap_or(record.meta.created_ms);
            let body = Body::new()
                .copy(old, "provider")
                .copy(old, "account_id")
                .copy(old, "title")
                .copy(old, "body")
                .copy(old, "raw_payload")
                .copy(old, "status")
                .copy(old, "importance_score")
                .copy(old, "triage_action")
                .copy(old, "triage_reason")
                .set("received_ms", received_ms)
                .set_some("scored_ms", time_ms(old, "scored_at"))
                .build();
            Ok(TargetRecord::new(NOTIFICATIONS, key_of(&record.id)?, body)
                .created_ms(record.meta.created_ms)
                .source_metadata(record.meta))
        })
        .collect()
}

/// A rename that keeps the body (dedup windows and provider settings).
fn renamed(
    collection: &'static str,
) -> impl Fn(Vec<SourceRecord>) -> Result<Vec<TargetRecord>> + Send + Sync + 'static {
    move |batch| {
        batch
            .into_iter()
            .map(|record| {
                Ok(
                    TargetRecord::new(collection, key_of(&record.id)?, record.body)
                        .created_ms(record.meta.created_ms)
                        .source_metadata(record.meta),
                )
            })
            .collect()
    }
}

/// `(workspace, event id)` of a v1 id, `<len>:<workspace>/<event>`.
fn split_legacy_core_id(id: &str) -> Option<(&str, &str)> {
    let (len, rest) = id.split_once(':')?;
    let len: usize = len.parse().ok()?;
    let workspace = rest.get(..len)?;
    let event = rest.get(len..)?.strip_prefix('/')?;
    Some((workspace, event))
}

fn core_v2(batch: Vec<SourceRecord>) -> Result<Vec<TargetRecord>> {
    Ok(batch
        .into_iter()
        .filter_map(|record| {
            let old = &record.body;
            let (workspace, event) = split_legacy_core_id(&record.id)?;
            let created_ms = time_ms(old, "created_at").unwrap_or(record.meta.created_ms);
            let body = Body::new()
                .set("workspace", workspace)
                .copy(old, "payload")
                .copy(old, "timestamp_ms")
                .set(
                    "read",
                    old.get("read").and_then(Value::as_bool).unwrap_or(false),
                )
                .build();
            Some(
                TargetRecord::new(CORE, core_id(workspace, event), body)
                    .created_ms(created_ms)
                    .source_metadata(record.meta),
            )
        })
        .collect())
}

#[cfg(test)]
#[path = "storage_schema_tests.rs"]
mod tests;
