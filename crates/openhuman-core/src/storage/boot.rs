//! Prepare the host's known storage scopes before installing a backend.
//!
//! Tiny Storage Drivers v2 has no global schema registry or scope enumerator.
//! Collection declarations belong to each record owner. The host asks each
//! linked crate for its declarations and prepares them on the shared local and
//! cluster scopes; tenant scopes are prepared when their owner is created.
//!
//! Preparing a scope also runs the tinyagents graph and harness owner-local
//! migrations (`migrate_owners`), which carry their v1 collection names and
//! shapes forward. Both are idempotent, so a prepared scope re-runs nothing.

use std::sync::Arc;

use tinystoragedrivers::{Scope, Storage, StorageBackend, StorageError};

/// What a boot preparation run did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BootReport {
    /// Known scopes prepared.
    pub scopes: usize,
    /// Collection declarations ensured across those scopes.
    pub collections: usize,
    /// Owner migrations that failed (logged; the next boot retries them).
    pub failures: usize,
}

/// Ensures each linked crate's collections in the host's shared scopes.
///
/// Per-profile and per-agent scopes are prepared when their owner is created;
/// no driver-specific scope enumeration is required or used.
///
/// # Errors
/// Returns a collection validation, scope binding, or preparation error.
/// Preparation is idempotent and may be retried on the next boot. A failed
/// owner migration is logged and counted in [`BootReport::failures`] instead,
/// so one scope's migration never stops the boot.
pub async fn prepare_and_migrate(
    backend: &Arc<dyn StorageBackend>,
) -> Result<BootReport, StorageError> {
    let owners = super::schema::collection_owners()?;
    let scopes = [Scope::new("local")?, Scope::new("cluster")?];
    let mut report = BootReport::default();
    for scope in scopes {
        let storage = backend.for_scope(&scope)?;
        for owner in &owners {
            for collection in &owner.collections {
                storage.ensure_collection(collection).await?;
                report.collections += 1;
            }
        }
        if let Err(error) = migrate_owners(&storage, &scope).await {
            tracing::warn!(
                target: "openhuman::storage",
                scope = scope.as_str(),
                %error,
                "[storage][boot] owner migration failed; retried at the next boot"
            );
            report.failures += 1;
        }
        report.scopes += 1;
    }
    tracing::info!(
        target: "openhuman::storage",
        scopes = report.scopes,
        collections = report.collections,
        failures = report.failures,
        "[storage][boot] owner collections prepared"
    );
    Ok(report)
}

/// Prepares a tenant or module scope before its owner starts writing there,
/// including the graph and harness owner migrations (`migrate_owners`).
///
/// # Errors
/// Returns the first owner declaration, migration or backend error.
pub async fn prepare_scope(storage: &Storage, scope: &Scope) -> Result<usize, StorageError> {
    let owners = super::schema::collection_owners()?;
    let mut count = 0;
    for owner in &owners {
        for collection in &owner.collections {
            storage.ensure_collection(collection).await?;
            count += 1;
        }
    }
    migrate_owners(storage, scope).await?;
    Ok(count)
}

/// Runs the tinyagents harness owner-local migrations on `scope`: v1 run
/// status, journals, blobs and key-value stores move to their v2 names.
/// Idempotent. (The graph owner is gone with `tinyagents-graph`.)
async fn migrate_owners(storage: &Storage, scope: &Scope) -> Result<(), StorageError> {
    let scoped = tinyagents_storage_kit::ScopedStorage::new(storage.clone(), scope.clone());
    tinyagents_harness::storage::migrate_storage(scoped).await
}

/// Runs the harness owner migrations on `scope` again after an
/// importer copied v1 records into it (the scope move). Their steps already
/// finished on the then-empty scope when it was prepared, and a finished step
/// is skipped, so the two owners' progress records are cleared first. The
/// steps write absent-only, so re-running over migrated records changes
/// nothing.
///
/// # Errors
///
/// The first storage error; the cleared steps re-run on the next preparation.
pub async fn remigrate_owners(storage: &Storage, scope: &Scope) -> Result<(), StorageError> {
    use tinyagents_storage_kit::{DocumentStoreExt, MIGRATIONS_COLLECTION};
    use tinystoragedrivers::{CollectionSpec, Precondition, Query};
    let docs = storage.documents();
    docs.ensure_collection(&CollectionSpec::new(MIGRATIONS_COLLECTION))
        .await?;
    for progress in docs.query_all(MIGRATIONS_COLLECTION, &Query::all()).await? {
        let owner = progress.doc.get("owner").and_then(|value| value.as_str());
        if matches!(owner, Some("harness")) {
            docs.delete(MIGRATIONS_COLLECTION, &progress.id, Precondition::None)
                .await?;
        }
    }
    rerun_harness_steps(storage, scope).await
}

/// Re-runs the harness schema steps under the owner's migration lease.
///
/// `open_storage` remembers a scope it already migrated in this process and
/// returns at once, so it cannot carry rows copied in after preparation. The
/// lease-taking primitive it is built on runs the steps again.
async fn rerun_harness_steps(storage: &Storage, scope: &Scope) -> Result<(), StorageError> {
    use tinyagents_storage_kit::{MigrateOptions, Registry};
    let scoped = tinyagents_storage_kit::ScopedStorage::new(storage.clone(), scope.clone());
    let owned = scoped.clone();
    tinyagents_storage_kit::with_migration_lease(&scoped, "harness", move |lease| async move {
        let registry = Registry::compose([tinyagents_harness::storage_schema()])?;
        registry.migrate(&owned, &MigrateOptions::new()).await?;
        lease.renew().await
    })
    .await
}

#[cfg(test)]
#[path = "boot_tests.rs"]
mod tests;
