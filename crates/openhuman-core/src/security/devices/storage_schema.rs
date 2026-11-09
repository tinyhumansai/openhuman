//! The `devices` owner's storage schema: the paired-device collection and the
//! step that brings the v1 `paired_devices` rows forward.
//!
//! | v1 | v2 | scope |
//! |---|---|---|
//! | `paired_devices` (`created_at`, `last_seen_at` RFC 3339) | `devices.paired` (`created_ms`, `last_seen_ms`) | tenant |

use anyhow::Result;
use serde_json::Value;
use tinystoragedrivers::{CollectionSpec, IndexSpec};

use crate::storage::schema_support::{
    key_of, map_legacy_collection, time_ms, Body, LegacySourceRecord as SourceRecord,
    LegacyTargetRecord as TargetRecord,
};

/// The v2 collection: one document per `channel_id`.
pub(super) const DEVICES: &str = "devices.paired";
/// The v1 name, still written by the legacy `devices.db` importer.
pub(super) const LEGACY_DEVICES: &str = "paired_devices";
/// The body version of [`DEVICES`].
///
/// Converts legacy paired-device rows into the current owner shape.
pub(crate) fn upgrade_records(
    records: Vec<crate::storage::import::Record>,
) -> Result<Vec<crate::storage::import::Record>> {
    map_legacy_collection(records, LEGACY_DEVICES, paired_v2)
}

/// The v2 collection with its index.
pub(super) fn spec() -> CollectionSpec {
    CollectionSpec::new(DEVICES).index(IndexSpec::new("by_active", ["revoked", "created_ms"]))
}

/// Collections owned by the paired-device store.
pub(crate) fn collection_specs() -> Vec<CollectionSpec> {
    vec![spec()]
}

/// The v1 collection as the importer declares it.
pub(super) fn legacy_collections() -> Vec<CollectionSpec> {
    vec![CollectionSpec::new(LEGACY_DEVICES)
        .index(IndexSpec::new("by_active", ["revoked", "created_at"]))]
}

fn paired_v2(batch: Vec<SourceRecord>) -> Result<Vec<TargetRecord>> {
    batch
        .into_iter()
        .map(|record| {
            let old = &record.body;
            let created_ms = time_ms(old, "created_at").unwrap_or(record.meta.created_ms);
            let body = Body::new()
                .copy(old, "label")
                .copy(old, "device_pubkey")
                .copy(old, "core_session_token_hash")
                .set("created_ms", created_ms)
                .set_some("last_seen_ms", time_ms(old, "last_seen_at"))
                .set(
                    "revoked",
                    old.get("revoked").and_then(Value::as_bool).unwrap_or(false),
                )
                .build();
            Ok(TargetRecord::new(DEVICES, key_of(&record.id)?, body)
                .created_ms(record.meta.created_ms)
                .source_metadata(record.meta))
        })
        .collect()
}

#[cfg(test)]
#[path = "storage_schema_tests.rs"]
mod tests;
