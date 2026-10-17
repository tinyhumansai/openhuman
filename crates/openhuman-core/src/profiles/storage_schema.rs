//! The `profiles` owner's storage schema: the registry of provisioned
//! profiles, kept in the shared `cluster` scope.
//!
//! | v1 | v2 | scope |
//! |---|---|---|
//! | `profiles` (`{profile_id, created_at (seconds), layout_version}`) | `profiles.registry` (`{profile_id, layout_version}`; creation time in tsd metadata) | cluster |
//!
//! The cluster scope is not a tenant, so boot's per-tenant migration never
//! visits it; [`migrate_cluster`] runs this owner's steps over it, the first
//! time a registry opens the shared backend.

use serde_json::Value;
use tinystoragedrivers::schema::{
    MigrateOptions, Registry, Schema, ScopeClass, SourceRecord, Step, TargetRecord,
};
use tinystoragedrivers::{CollectionSpec, Result, ScopedStorage};

use crate::storage::schema_support::{key_of, owner, Body};

/// The v2 collection: one document per profile.
pub const REGISTRY_COLLECTION: &str = "profiles.registry";
/// The v1 name.
pub(super) const LEGACY_REGISTRY_COLLECTION: &str = "profiles";
const SCHEMA: u32 = 2;

/// What the profile registry keeps and the step from the v1 collection.
#[must_use]
pub fn storage_schema() -> Schema {
    Schema::new(owner("profiles"))
        .collection_with(
            SCHEMA,
            CollectionSpec::new(REGISTRY_COLLECTION),
            ScopeClass::Cluster,
        )
        .step(
            Step::new("registry_v2", REGISTRY_COLLECTION, 1, SCHEMA, registry_v2)
                .source(LEGACY_REGISTRY_COLLECTION),
        )
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
pub async fn migrate_cluster(scoped: &ScopedStorage) -> Result<()> {
    let registry = Registry::compose([storage_schema()])?;
    scoped
        .documents()
        .ensure_collection(&CollectionSpec::new(REGISTRY_COLLECTION))
        .await?;
    registry.migrate(scoped, &MigrateOptions::new()).await?;
    Ok(())
}

#[cfg(test)]
#[path = "storage_schema_tests.rs"]
mod tests;
