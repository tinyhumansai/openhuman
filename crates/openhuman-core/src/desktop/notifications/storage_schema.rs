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

use serde_json::Value;
use tinystoragedrivers::schema::{Schema, ScopeClass, SourceRecord, Step, TargetRecord};
use tinystoragedrivers::{CollectionSpec, IndexSpec, Key, Result};

use crate::storage::schema_support::{key_of, owner, time_ms, Body};

pub(super) const NOTIFICATIONS: &str = "notifications.integration";
pub(super) const DEDUP: &str = "notifications.dedup";
pub(super) const SETTINGS: &str = "notifications.settings";
pub(super) const CORE: &str = "notifications.core";

pub(super) const LEGACY_NOTIFICATIONS: &str = "integration_notifications";
pub(super) const LEGACY_DEDUP: &str = "notification_dedup";
pub(super) const LEGACY_SETTINGS: &str = "notification_settings";
pub(super) const LEGACY_CORE: &str = "core_notifications";

const SCHEMA: u32 = 2;

/// What the notification store keeps and the steps from the v1 collections.
#[must_use]
pub fn storage_schema() -> Schema {
    let mut schema = Schema::new(owner("notifications"));
    for spec in specs() {
        schema = schema.collection_with(SCHEMA, spec, ScopeClass::Tenant);
    }
    schema
        .step(
            Step::new("integration_v2", NOTIFICATIONS, 1, SCHEMA, integration_v2)
                .source(LEGACY_NOTIFICATIONS),
        )
        .step(Step::new("dedup_v2", DEDUP, 1, SCHEMA, renamed(DEDUP)).source(LEGACY_DEDUP))
        .step(
            Step::new("settings_v2", SETTINGS, 1, SCHEMA, renamed(SETTINGS))
                .source(LEGACY_SETTINGS),
        )
        .step(Step::new("core_v2", CORE, 1, SCHEMA, core_v2).source(LEGACY_CORE))
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
    Key::of([workspace, event_id])
        .expect("a two-part key is valid")
        .as_str()
        .to_owned()
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
                .created_ms(record.meta.created_ms))
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
                        .created_ms(record.meta.created_ms),
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
            Some(TargetRecord::new(CORE, core_id(workspace, event), body).created_ms(created_ms))
        })
        .collect())
}

#[cfg(test)]
#[path = "storage_schema_tests.rs"]
mod tests;
