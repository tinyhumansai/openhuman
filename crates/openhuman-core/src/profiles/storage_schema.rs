//! The `profiles` owner's storage schema: the registry of provisioned
//! profiles, kept in the shared `cluster` scope.
//!
//! | v1 | v2 | scope |
//! |---|---|---|
//! | `profiles` (`{profile_id, created_at (seconds), layout_version}`) | `profiles.registry` (`{profile_id, layout_version}`; creation time in tsd metadata) | cluster |
//! | `leases` (`{owner, endpoint, epoch, expires_at_ms, released}`) | host lease collection (`{holder, epoch, expires_ms, endpoint?, released?}`) | cluster |
//!
//! The cluster scope is not a tenant, so boot's per-tenant migration never
//! visits it; [`migrate_cluster`] ensures this owner's declaration when a
//! registry opens the shared backend. Legacy lease conversion is exposed as a
//! typed import transform for the host importer; opening a profile registry
//! never mutates a legacy collection.

use anyhow::Result;
use serde_json::Value;
use tinystoragedrivers::{CollectionSpec, Storage};

use crate::storage::schema_support::{
    key_of, key_of_parts, map_legacy_collection, Body, LegacySourceRecord as SourceRecord,
    LegacyTargetRecord as TargetRecord,
};

/// The v2 collection: one document per profile.
pub const REGISTRY_COLLECTION: &str = "profiles.registry";
/// The v1 name.
pub(super) const LEGACY_REGISTRY_COLLECTION: &str = "profiles";

/// Collections owned by the profile registry.
pub(crate) fn collection_specs() -> Vec<CollectionSpec> {
    vec![CollectionSpec::new(REGISTRY_COLLECTION)]
}

/// Converts legacy profile records into the current registry shape.
pub(crate) fn upgrade_records(
    records: Vec<crate::storage::import::Record>,
) -> Result<Vec<crate::storage::import::Record>> {
    map_legacy_collection(records, LEGACY_REGISTRY_COLLECTION, registry_v2)
}

fn registry_v2(batch: Vec<SourceRecord>) -> Result<Vec<TargetRecord>> {
    batch
        .into_iter()
        .map(|record| {
            let old = &record.body;
            let body = Body::new()
                .copy(old, "profile_id")
                .set(
                    "layout_version",
                    old.get("layout_version")
                        .and_then(Value::as_u64)
                        .unwrap_or_default(),
                )
                .build();
            // Unix seconds become the record's creation time.
            let created_ms = old
                .get("created_at")
                .and_then(Value::as_i64)
                .map_or(record.meta.created_ms, |secs| secs.saturating_mul(1000));
            Ok(
                TargetRecord::new(REGISTRY_COLLECTION, key_of(&record.id)?, body)
                    .created_ms(created_ms),
            )
        })
        .collect()
}

/// Runs this owner's steps over the cluster scope `scoped`. Idempotent: a
/// finished step is skipped.
///
/// # Errors
///
/// A storage error; the registry then reads the unmigrated records as absent
/// until the next open retries.
pub async fn migrate_cluster(scoped: &Storage) -> Result<()> {
    scoped
        .documents()
        .ensure_collection(&CollectionSpec::new(REGISTRY_COLLECTION))
        .await?;
    Ok(())
}

/// Converts legacy lease documents to the host's typed lease import records.
/// Invalid source rows fail closed so the importer cannot silently discard a
/// live lease and admit an unfenced writer.
pub(crate) fn upgrade_lease_records(
    records: Vec<crate::storage::import::Record>,
) -> Result<Vec<crate::storage::import::Record>> {
    map_legacy_collection(records, "leases", |batch| {
        batch
            .into_iter()
            .map(|record| {
                let old = record.body;
                let owner = old
                    .get("owner")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("legacy lease has no owner"))?;
                let epoch = old
                    .get("epoch")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| anyhow::anyhow!("legacy lease has no valid epoch"))?;
                let expires = old
                    .get("expires_at_ms")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| anyhow::anyhow!("legacy lease has no valid expiry"))?;
                let mut body = serde_json::Map::new();
                body.insert("holder".into(), owner.into());
                body.insert("epoch".into(), epoch.into());
                body.insert("expires_ms".into(), expires.into());
                if let Some(endpoint) = old.get("endpoint").and_then(Value::as_str) {
                    body.insert("endpoint".into(), endpoint.into());
                }
                if old.get("released").and_then(Value::as_bool) == Some(true) {
                    body.insert("released".into(), true.into());
                }
                Ok(TargetRecord::new(
                    "openhuman.leases",
                    key_of_parts(["profiles", record.id.as_str()])?,
                    Value::Object(body),
                )
                .source_metadata(record.meta))
            })
            .collect()
    })
}

#[cfg(test)]
#[path = "storage_schema_tests.rs"]
mod tests;
