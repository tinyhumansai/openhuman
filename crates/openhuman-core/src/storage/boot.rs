//! Boot-time preparation of a freshly opened backend.
//!
//! [`prepare_and_migrate`] runs, in order, and logs each step:
//!
//! 1. **prepare** — declare every owner's collections from the composed
//!    registry ([`super::schema`]);
//! 2. **scope move** — copy v1 scopes to the one scope grammar
//!    ([`super::scope_move`]), because an owner step runs inside one scope;
//! 3. per scope the backend holds (always `local`): the **pre-migrate**
//!    importer (`import_legacy_ledger`), then `Registry::migrate`, then the
//!    **post-migrate** importers that move key-value state and counters into
//!    `tsd.kv` / `tsd.counters` (a document step cannot write another owner's
//!    collections).
//!
//! Every piece is resumable and idempotent (progress lives in each scope's
//! `tsd.migrations`), so a crash anywhere re-runs from the last finished
//! batch. A failure in one scope is logged and counted, never silently
//! dropped, and does not stop the other scopes; the next boot retries it.
//! Scopes created after boot (a new profile, a new agent) hold no v1 data and
//! need no run.

use std::sync::Arc;

use tinystoragedrivers::schema::{MigrateOptions, Registry};
use tinystoragedrivers::{Capability, Scope, StorageBackend, StorageError};

/// What a boot run did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BootReport {
    /// Scopes migrated (including ones with nothing to do).
    pub scopes: usize,
    /// Migration steps that ran to completion.
    pub steps_run: usize,
    /// `(scope, step, message)` for every piece that failed.
    pub failures: Vec<(String, &'static str, String)>,
}

/// Prepares `backend` and migrates every scope it holds. See the module docs.
///
/// # Errors
///
/// When the registry does not compose or the backend cannot be prepared or
/// its scopes cannot be moved; a failure inside one scope's migration is
/// recorded in [`BootReport::failures`] instead.
pub async fn prepare_and_migrate(
    backend: &Arc<dyn StorageBackend>,
) -> Result<BootReport, StorageError> {
    let registry = super::schema::registry()?;
    tracing::debug!(target: "openhuman::storage", "[storage][boot] preparing the composed schema");
    registry.prepare(backend.as_ref()).await?;

    super::scope_move::run(backend).await?;

    let mut report = BootReport::default();
    for scope in scopes_to_migrate(backend).await {
        report.scopes += 1;
        migrate_scope(registry, backend, &scope, &mut report).await;
    }
    if report.failures.is_empty() {
        tracing::info!(
            target: "openhuman::storage",
            scopes = report.scopes,
            steps = report.steps_run,
            "[storage][boot] schema prepared and migrated"
        );
    } else {
        tracing::warn!(
            target: "openhuman::storage",
            failures = report.failures.len(),
            "[storage][boot] some migrations failed and will be retried at the next boot"
        );
    }
    Ok(report)
}

/// Runs `owner`'s steps again over `scope` after a legacy-file importer wrote
/// v1-shaped records there. The steps finished at boot (or an earlier import),
/// and a finished step is skipped, so the owner's progress records are cleared
/// first; the steps are pure and write absent-only, so re-running over the
/// already migrated records changes nothing. Returns the steps that ran.
///
/// # Errors
///
/// The first storage error. The importer's records stay in their v1 form and
/// the next call retries.
pub async fn remigrate_owner(
    backend: &Arc<dyn StorageBackend>,
    scope: &Scope,
    owner: &str,
) -> Result<usize, StorageError> {
    use tinystoragedrivers::{DocumentStoreExt, Query};
    let registry = super::schema::registry()?;
    let scoped = backend.for_scope(scope)?;
    let docs = scoped.documents();
    docs.ensure_collection(&tinystoragedrivers::CollectionSpec::new(
        tinystoragedrivers::MIGRATIONS_COLLECTION,
    ))
    .await?;
    for progress in docs
        .query_all(tinystoragedrivers::MIGRATIONS_COLLECTION, &Query::all())
        .await?
    {
        if progress.doc.get("owner").and_then(|v| v.as_str()) == Some(owner)
            && progress.doc.get("to").and_then(|v| v.as_u64()).unwrap_or(0) > 0
        {
            docs.delete(
                tinystoragedrivers::MIGRATIONS_COLLECTION,
                &progress.id,
                tinystoragedrivers::Precondition::None,
            )
            .await?;
        }
    }
    let report = registry.migrate(&scoped, &MigrateOptions::new()).await?;
    tracing::info!(
        target: "openhuman::storage",
        scope = scope.as_str(),
        owner,
        steps = report.ran.len(),
        "[storage][boot] re-ran an owner's steps after a legacy import"
    );
    Ok(report.ran.len())
}

/// `local`, plus every v2 scope the backend reports (when it can enumerate).
async fn scopes_to_migrate(backend: &Arc<dyn StorageBackend>) -> Vec<Scope> {
    let mut scopes = vec![Scope::local()];
    if backend
        .capabilities()
        .contains(Capability::ScopeEnumeration)
    {
        match backend.scopes("").await {
            Ok(found) => {
                for scope in found {
                    if scope.parsed().is_ok() && !scope.is_cluster() && !scopes.contains(&scope) {
                        scopes.push(scope);
                    }
                }
            }
            Err(error) => tracing::warn!(
                target: "openhuman::storage",
                %error,
                "[storage][boot] could not list scopes; migrating the local scope only"
            ),
        }
    }
    scopes
}

async fn migrate_scope(
    registry: &Registry,
    backend: &Arc<dyn StorageBackend>,
    scope: &Scope,
    report: &mut BootReport,
) {
    let scoped = match backend.for_scope(scope) {
        Ok(scoped) => scoped,
        Err(error) => {
            record(report, scope, "bind", &error);
            return;
        }
    };
    // Pre-migrate importer: the run ledger's v1 leases and counters.
    if let Err(error) = tinyagents_session::import_legacy_ledger(&scoped).await {
        record(report, scope, "import_legacy_ledger", &error);
    }
    // Marker rows older builds wrote become `tsd.migrations` entries, so an
    // already imported profile is not imported again.
    if let Err(error) = super::import_state::migrate_legacy_markers(&scoped).await {
        record(report, scope, "migrate_legacy_markers", &error);
    }
    match registry.migrate(&scoped, &MigrateOptions::new()).await {
        Ok(done) => {
            if !done.ran.is_empty() {
                tracing::info!(
                    target: "openhuman::storage",
                    scope = scope.as_str(),
                    steps = done.ran.len(),
                    "[storage][boot] migration steps ran"
                );
            }
            report.steps_run += done.ran.len();
        }
        Err(error) => {
            record(report, scope, "migrate", &error);
            return;
        }
    }
    post_migrate_importers(&scoped, scope, report).await;
}

async fn post_migrate_importers(
    scoped: &tinystoragedrivers::ScopedStorage,
    scope: &Scope,
    report: &mut BootReport,
) {
    use tinyagents_harness::storage::{import_legacy_kv, LegacyKv};
    for (owner, legacy, kind) in [
        ("harness", "harness_store", LegacyKv::Flat),
        ("session", "session_kv", LegacyKv::Flat),
        ("harness", "harness_namespaced_store", LegacyKv::Namespaced),
        ("harness", "harness_response_cache", LegacyKv::ResponseCache),
    ] {
        if let Err(error) = import_legacy_kv(scoped, owner, legacy, kind).await {
            record(report, scope, "import_legacy_kv", &error);
        }
    }
    if let Err(error) = tinyagents_graph::import_legacy_leases(scoped).await {
        record(report, scope, "import_legacy_leases", &error);
    }
    if let Err(error) = tinyagents_graph::import_legacy_task_cache(scoped).await {
        record(report, scope, "import_legacy_task_cache", &error);
    }
    for (step, error) in super::host_importers::run(scoped).await {
        record(report, scope, step, &error);
    }
    #[cfg(feature = "flows")]
    {
        if let Err(error) = tinyflows_drivers::import_legacy_kv(scoped).await {
            record(report, scope, "flows_import_legacy_kv", &error);
        }
        if let Err(error) = tinyflows_drivers::import_legacy_cron_counters(scoped).await {
            record(report, scope, "import_legacy_cron_counters", &error);
        }
    }
}

fn record(
    report: &mut BootReport,
    scope: &Scope,
    step: &'static str,
    error: &dyn std::fmt::Display,
) {
    tracing::warn!(
        target: "openhuman::storage",
        scope = scope.as_str(),
        step,
        %error,
        "[storage][boot] step failed"
    );
    report
        .failures
        .push((scope.as_str().to_owned(), step, error.to_string()));
}

#[cfg(test)]
#[path = "boot_tests.rs"]
mod tests;
