//! Run-journal sealing and periodic storage maintenance (epic #7973, #7995).
//!
//! A turn seals its journal stream (`harness.journal/<run_id>`) when it ends
//! (`agent::tinyagents::journal`). This module owns the rest of the lifecycle:
//!
//! - a **sweep** on a fixed tick: `Registry::maintain_all` applies every
//!   owner's seal and retention policy (deleting journals sealed longer ago
//!   than `[storage] journal_retention_days`), then `StorageBackend::maintain`
//!   returns the freed space;
//! - a **one-shot backfill** per scope that seals the unsealed journal of
//!   every run that is no longer active. It is recorded in that scope's
//!   `tsd.migrations` (`harness` / [`BACKFILL_STEP`]); sealing is idempotent,
//!   so a crash part-way simply re-runs the pass at the next sweep.
//!
//! Best-effort: a failure is logged and retried on the next tick.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use tinyagents_harness::observability::DriverStatusStore;
use tinyagents_harness::HarnessStatusStore;
use tinystoragedrivers::schema::{Item, Migrations, Registry};
use tinystoragedrivers::{
    Capability, CollectionSpec, MaintenanceBudget, Scope, StorageBackend, StorageError,
    MIGRATIONS_COLLECTION,
};

/// The journal stream family the harness declares.
pub const JOURNAL_STREAM: &str = "harness.journal";

/// Owner and step of the backfill's `tsd.migrations` record.
pub const BACKFILL_OWNER: &str = "harness";
/// Step id of the backfill's `tsd.migrations` record.
pub const BACKFILL_STEP: &str = "journal_seal_backfill_v1";

/// How often the sweep wakes.
pub const SWEEP_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// Delay before the first sweep, after the boot reaper and migrations.
pub const FIRST_SWEEP_DELAY: Duration = Duration::from_secs(10 * 60);

const LOG: &str = "[storage::journal_maintenance]";
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// The composed registry with the journal retention set to `days` (`0` keeps
/// sealed journals forever).
///
/// # Errors
///
/// The composition error, as [`super::schema::registry`].
pub fn registry_with_retention(days: u32) -> Result<Registry, StorageError> {
    let mut schemas = super::schema::owner_schemas();
    for schema in &mut schemas {
        for item in &mut schema.items {
            if let Item::Stream { name, seal, .. } = item {
                if name == JOURNAL_STREAM {
                    seal.retain_ms = (days > 0).then(|| u64::from(days) * DAY_MS);
                }
            }
        }
    }
    Registry::compose(schemas).map_err(|error| StorageError::invalid_input(error.to_string()))
}

/// What one maintenance pass did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MaintenanceSummary {
    /// Journals sealed by the backfill in this pass.
    pub backfilled: u64,
    /// Streams the policies deleted (retention passed).
    pub deleted: u64,
    /// Streams the policies sealed.
    pub sealed: u64,
}

/// One pass at `now_ms`: backfill, policies, then the backend's own
/// maintenance.
///
/// # Errors
///
/// The policy or backend error; the backfill logs its own failures.
pub async fn maintain_once(
    backend: &Arc<dyn StorageBackend>,
    retention_days: u32,
    now_ms: i64,
) -> Result<MaintenanceSummary, StorageError> {
    let mut summary = MaintenanceSummary::default();
    for scope in scopes(backend).await {
        match backfill_scope(backend.as_ref(), &scope).await {
            Ok(sealed) => summary.backfilled += sealed,
            Err(error) => tracing::warn!(
                scope = scope.as_str(),
                %error,
                "{LOG} journal seal backfill failed; retrying on the next sweep"
            ),
        }
    }
    let registry = registry_with_retention(retention_days)?;
    let budget = MaintenanceBudget::unlimited();
    let report = if backend
        .capabilities()
        .contains(Capability::ScopeEnumeration)
    {
        registry
            .maintain_all(backend.as_ref(), now_ms, budget)
            .await?
    } else {
        registry
            .maintain(&backend.for_scope(&Scope::local())?, now_ms, budget)
            .await?
    };
    summary.deleted = report.deleted;
    summary.sealed = report.sealed;
    backend.maintain(budget).await?;
    tracing::debug!(
        backfilled = summary.backfilled,
        sealed = summary.sealed,
        deleted = summary.deleted,
        "{LOG} pass complete"
    );
    Ok(summary)
}

/// `local` plus every scope the backend can list.
async fn scopes(backend: &Arc<dyn StorageBackend>) -> Vec<Scope> {
    let mut scopes = vec![Scope::local()];
    if backend
        .capabilities()
        .contains(Capability::ScopeEnumeration)
    {
        if let Ok(found) = backend.scopes("").await {
            for scope in found {
                if scope.parsed().is_ok() && !scope.is_cluster() && !scopes.contains(&scope) {
                    scopes.push(scope);
                }
            }
        }
    }
    scopes
}

/// Seals every unsealed run journal in `scope` whose run is not active, once:
/// the pass is recorded in the scope's `tsd.migrations`. Returns how many
/// journals it sealed (0 when already done).
///
/// # Errors
///
/// A storage error; the record is written only after the whole pass, so the
/// next call resumes by skipping what is already sealed.
pub async fn backfill_scope(
    backend: &dyn StorageBackend,
    scope: &Scope,
) -> Result<u64, StorageError> {
    let storage = backend.for_scope(scope)?;
    storage
        .documents()
        .ensure_collection(&CollectionSpec::new(MIGRATIONS_COLLECTION))
        .await?;
    let migrations = Migrations::new(&storage);
    if migrations.done(BACKFILL_OWNER, BACKFILL_STEP).await? {
        return Ok(0);
    }
    let active: HashSet<String> = DriverStatusStore::new(Arc::clone(storage.documents()))
        .list_active()
        .await
        .map_err(|error| StorageError::invalid_input(error.to_string()))?
        .into_iter()
        .map(|status| status.run_id.as_str().to_string())
        .collect();
    let streams = storage.streams();
    let family = format!("{JOURNAL_STREAM}/");
    let mut sealed = 0;
    for name in streams.streams(JOURNAL_STREAM).await? {
        let Some(run) = name.strip_prefix(&family) else {
            continue;
        };
        if active.contains(run) {
            continue;
        }
        match streams.info(&name).await? {
            Some(info) if info.sealed_ms.is_none() => {
                streams.seal(&name).await?;
                sealed += 1;
            }
            _ => {}
        }
    }
    migrations.mark_done(BACKFILL_OWNER, BACKFILL_STEP).await?;
    tracing::info!(
        scope = scope.as_str(),
        sealed,
        "{LOG} journal seal backfill done"
    );
    Ok(sealed)
}

/// Retention days from the current config; the default when it cannot load.
async fn configured_retention_days(ctx: &crate::core::runtime::DispatchCtx) -> u32 {
    match crate::config::ops::load_current_or_init(ctx).await {
        Ok(config) => config.storage.journal_retention_days,
        Err(error) => {
            tracing::debug!(%error, "{LOG} config unavailable; using the default retention");
            crate::config::schema::StorageConfig::default().journal_retention_days
        }
    }
}

/// Spawns the sweep: after `first_delay`, then every `interval`, until aborted.
pub fn spawn(
    ctx: &crate::core::runtime::DispatchCtx,
    first_delay: Duration,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    let task_ctx = ctx.clone();
    ctx.spawn(async move {
        tracing::debug!(
            first_delay_secs = first_delay.as_secs(),
            interval_secs = interval.as_secs(),
            "{LOG} scheduled"
        );
        tokio::time::sleep(first_delay).await;
        let mut tick = tokio::time::interval(interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let Some(backend) = super::installed() else {
                continue;
            };
            let days = configured_retention_days(&task_ctx).await;
            let now_ms = chrono::Utc::now().timestamp_millis();
            if let Err(error) = maintain_once(&backend, days, now_ms).await {
                tracing::warn!(%error, "{LOG} pass failed; retrying on the next tick");
            }
        }
    })
}

#[cfg(test)]
#[path = "journal_maintenance_tests.rs"]
mod tests;
