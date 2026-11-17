//! Host scheduling and per-scope backfill state for harness journal maintenance.
//!
//! Journal lifecycle rules belong to TinyAgents. This module supplies the
//! host's retention setting and clock, visits scopes the host knows about,
//! and records a durable per-scope marker after the owner's one-shot inactive
//! journal backfill succeeds.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use tinyagents_harness::observability::{is_active, DriverStatusStore, HarnessStatusStore};
use tinyagents_harness::store::DriverAppendStore;
use tinystoragedrivers::{CollectionSpec, ErrorKind, Precondition, StorageBackend};

use crate::storage::Scope;

/// How often the sweep wakes.
pub const SWEEP_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// Delay before the first sweep, after boot preparation and migrations.
pub const FIRST_SWEEP_DELAY: Duration = Duration::from_secs(10 * 60);

/// Host-owned durable marker collection for the one-shot journal backfill.
pub const BACKFILL_COLLECTION: &str = "openhuman.journal_backfill";

const BACKFILL_VERSION: u64 = 1;
const LOG: &str = "[storage::journal_maintenance]";
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct JournalMaintenanceReport {
    backfilled: usize,
    deleted: usize,
    conflicts: usize,
}

/// Host collection storing the successful one-shot backfill marker per scope.
pub(crate) fn collection_specs() -> Vec<CollectionSpec> {
    vec![CollectionSpec::new(BACKFILL_COLLECTION)]
}

/// One pass over the scopes known to this host.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MaintenanceSummary {
    /// Journals sealed by the inactive-run backfill.
    pub backfilled: u64,
    /// Expired sealed journals removed by TinyAgents.
    pub deleted: u64,
    /// Eligible journals that changed before conditional deletion.
    pub conflicts: u64,
    /// Scopes whose maintenance pass completed.
    pub scopes: u64,
}

/// Runs owner journal maintenance once in every scope known to the host.
///
/// Owner policy stays in TinyAgents; OpenHuman supplies a retention duration,
/// timestamp, and a per-scope one-shot backfill decision. A failed owner pass
/// does not write the marker, so it is retried on the next sweep.
pub async fn maintain_once(
    backend: &Arc<dyn StorageBackend>,
    retention_days: u32,
    now_ms: i64,
) -> Result<MaintenanceSummary, crate::storage::StorageError> {
    let retention_ms = u64::from(retention_days).saturating_mul(DAY_MS);
    let work = known_scope_work(backend)?;
    maintain_work_items(work, retention_ms, now_ms).await
}

struct ScopeWorkItem {
    backend: Arc<dyn StorageBackend>,
    scope: Scope,
    // Retains the open host slot. The captured backend separately pins the
    // exact lease epoch and carries its condition on every mutation.
    _profile: Option<Arc<crate::profiles::Profile>>,
}

fn known_scope_work(
    backend: &Arc<dyn StorageBackend>,
) -> Result<Vec<ScopeWorkItem>, crate::storage::StorageError> {
    let mut base_scopes = BTreeSet::new();
    if !crate::core::runtime::mode::is_saas() {
        base_scopes.insert(Scope::local());
    }

    for (agent, context) in crate::storage::agents::contexts() {
        let dispatch = crate::core::runtime::DispatchCtx::new(context);
        if let Ok(tenant) = dispatch.tenant() {
            if tenant.profile.is_none() {
                if let Some(agent) = tenant.agent.or(Some(agent)) {
                    base_scopes.insert(crate::storage::scope_for_agent(&agent));
                }
            }
        }
    }

    if !crate::core::runtime::mode::is_saas() {
        for module in crate::modules::registry::ALL {
            if let Ok(scope) = Scope::local().module(module.id) {
                base_scopes.insert(scope);
            }
        }
    }

    let mut work = base_scopes
        .into_iter()
        .map(|scope| ScopeWorkItem {
            backend: Arc::clone(backend),
            scope,
            _profile: None,
        })
        .collect::<Vec<_>>();

    if let Some(host) = crate::profiles::host::host() {
        work.extend(captured_profile_work(
            host.as_ref(),
            host.hosted_profiles(),
            super::fence::system_now_ms(),
        )?);
    }
    Ok(work)
}

fn captured_profile_work(
    host: &crate::profiles::ProfileHost,
    profiles: Vec<Arc<crate::profiles::Profile>>,
    now_ms: u64,
) -> Result<Vec<ScopeWorkItem>, crate::storage::StorageError> {
    let mut work = Vec::new();
    for profile in profiles {
        if profile.lease_fence().check_local(now_ms).is_err() {
            continue;
        }
        let Some(profile_backend) = host.storage_backend_for(&profile)? else {
            // File leases have no shared storage backend to maintain.
            continue;
        };
        let base = crate::storage::scope_for_profile(profile.id.as_str());
        let mut profile_scopes = BTreeSet::from([base.clone()]);
        for module in crate::modules::registry::ALL {
            if let Ok(scope) = base.module(module.id) {
                profile_scopes.insert(scope);
            }
        }
        work.extend(profile_scopes.into_iter().map(|scope| ScopeWorkItem {
            backend: Arc::clone(&profile_backend),
            scope,
            _profile: Some(Arc::clone(&profile)),
        }));
    }
    Ok(work)
}

async fn maintain_work_items(
    work: Vec<ScopeWorkItem>,
    retention_ms: u64,
    now_ms: i64,
) -> Result<MaintenanceSummary, crate::storage::StorageError> {
    let mut summary = MaintenanceSummary::default();
    for item in work {
        match maintain_scope(&item.backend, &item.scope, retention_ms, now_ms).await {
            Ok(report) => {
                summary.scopes += 1;
                summary.backfilled += report.backfilled as u64;
                summary.deleted += report.deleted as u64;
                summary.conflicts += report.conflicts as u64;
            }
            Err(error) => tracing::warn!(
                scope = item.scope.as_str(),
                %error,
                "{LOG} owner journal maintenance failed; retrying on the next sweep"
            ),
        }
    }
    tracing::debug!(
        scopes = summary.scopes,
        backfilled = summary.backfilled,
        deleted = summary.deleted,
        conflicts = summary.conflicts,
        "{LOG} pass complete"
    );
    Ok(summary)
}

#[cfg(test)]
async fn maintain_scopes(
    backend: &Arc<dyn StorageBackend>,
    scopes: Vec<Scope>,
    retention_ms: u64,
    now_ms: i64,
) -> Result<MaintenanceSummary, crate::storage::StorageError> {
    let mut summary = MaintenanceSummary::default();
    for scope in scopes {
        match maintain_scope(backend, &scope, retention_ms, now_ms).await {
            Ok(report) => {
                summary.scopes += 1;
                summary.backfilled += report.backfilled as u64;
                summary.deleted += report.deleted as u64;
                summary.conflicts += report.conflicts as u64;
            }
            Err(error) => tracing::warn!(
                scope = scope.as_str(),
                %error,
                "{LOG} owner journal maintenance failed; retrying on the next sweep"
            ),
        }
    }
    tracing::debug!(
        scopes = summary.scopes,
        backfilled = summary.backfilled,
        deleted = summary.deleted,
        conflicts = summary.conflicts,
        "{LOG} pass complete"
    );
    Ok(summary)
}

async fn maintain_scope(
    backend: &Arc<dyn StorageBackend>,
    scope: &Scope,
    retention_ms: u64,
    now_ms: i64,
) -> Result<JournalMaintenanceReport, crate::storage::StorageError> {
    let storage = backend.for_scope(scope)?;
    let docs = Arc::clone(storage.documents());
    docs.ensure_collection(&CollectionSpec::new(BACKFILL_COLLECTION))
        .await?;
    let key = crate::storage::schema_support::key_of("inactive-journal-backfill-v1")?;
    let backfill_done = match docs.get(BACKFILL_COLLECTION, &key).await? {
        None => false,
        Some(record) => {
            let version = record
                .doc
                .get("version")
                .and_then(serde_json::Value::as_u64);
            if version != Some(BACKFILL_VERSION) {
                return Err(crate::storage::StorageError::invalid_input(format!(
                    "invalid journal backfill marker in scope {}",
                    scope.as_str()
                )));
            }
            true
        }
    };

    let report = maintain_run_journals(
        &DriverStatusStore::new(docs),
        storage.streams().as_ref(),
        retention_ms,
        now_ms,
        !backfill_done,
    )
    .await?;

    if !backfill_done {
        match storage
            .documents()
            .put_with(
                BACKFILL_COLLECTION,
                &key,
                serde_json::json!({ "version": BACKFILL_VERSION }),
                Precondition::Absent,
                Default::default(),
            )
            .await
        {
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::Conflict => {}
            Err(error) => return Err(error),
        }
    }
    Ok(report)
}

/// Host-owned retention and one-time sealing for harness event journals.
/// TinyAgents owns the event format; OpenHuman owns the lifecycle schedule.
async fn maintain_run_journals(
    statuses: &dyn HarnessStatusStore,
    streams: &dyn tinystoragedrivers::api::StreamStore,
    retention_ms: u64,
    now_ms: i64,
    backfill: bool,
) -> Result<JournalMaintenanceReport, crate::storage::StorageError> {
    let mut report = JournalMaintenanceReport::default();
    for name in streams.streams(DriverAppendStore::JOURNAL_PREFIX).await? {
        let Some(run_id) = name.strip_prefix(DriverAppendStore::JOURNAL_PREFIX) else {
            continue;
        };
        let status = statuses
            .get_status(run_id)
            .await
            .map_err(|error| crate::storage::StorageError::backend(error.to_string()))?;
        if backfill && !status.as_ref().is_some_and(is_active) {
            let already_sealed = streams
                .info(&name)
                .await?
                .is_some_and(|info| info.sealed_ms.is_some());
            if !already_sealed {
                streams.seal(&name).await?;
                report.backfilled += 1;
            }
        }
        if retention_ms == 0 {
            continue;
        }
        let Some(info) = streams.info(&name).await? else {
            continue;
        };
        let Some(sealed_ms) = info.sealed_ms else {
            continue;
        };
        if i128::from(now_ms) - i128::from(sealed_ms) < i128::from(retention_ms) {
            continue;
        }
        if streams.delete_sealed_stream(&name, sealed_ms).await? {
            report.deleted += 1;
        } else {
            report.conflicts += 1;
        }
    }
    Ok(report)
}

/// Scopes this process can safely maintain, plus the profile handles that
/// keep their lease fences alive for the duration of a pass.
///
/// This does not ask the backend to enumerate arbitrary scopes. In SaaS mode,
/// profile scopes are included only when this process currently holds their
/// profile handle and lease. Dormant profiles are maintained after this host
/// opens them; sweeping their scopes directly could mutate a profile leased
/// by another host. Cluster scope is intentionally excluded: harness run
/// journals have historically been tenant/local data, not cluster data.
#[cfg(test)]
fn known_scopes(
) -> Result<(Vec<Arc<crate::profiles::Profile>>, Vec<Scope>), crate::storage::StorageError> {
    let mut scopes = BTreeSet::new();
    if !crate::core::runtime::mode::is_saas() {
        scopes.insert(Scope::local());
    }

    let now_ms = u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(u64::MAX);
    let profiles = crate::profiles::host::host()
        .map(|host| host.hosted_profiles())
        .unwrap_or_default();
    let (hosted_profiles, profile_scopes) = profile_scope_snapshot(profiles, now_ms);
    scopes.extend(profile_scopes);

    for (agent, context) in crate::storage::agents::contexts() {
        let dispatch = crate::core::runtime::DispatchCtx::new(context);
        if let Ok(tenant) = dispatch.tenant() {
            if tenant.profile.is_none() {
                if let Some(agent) = tenant.agent.or(Some(agent)) {
                    scopes.insert(crate::storage::scope_for_agent(&agent));
                }
            }
        }
    }

    if !crate::core::runtime::mode::is_saas() {
        for module in crate::modules::registry::ALL {
            if let Ok(scope) = Scope::local().module(module.id) {
                scopes.insert(scope);
            }
        }
    }
    Ok((hosted_profiles, scopes.into_iter().collect()))
}

/// Retain only profile handles whose lease is still live and enumerate their
/// tenant/module scopes. Keeping the handles alive prevents release while the
/// caller is using those scopes.
#[cfg(test)]
fn profile_scope_snapshot(
    profiles: Vec<Arc<crate::profiles::Profile>>,
    now_ms: u64,
) -> (Vec<Arc<crate::profiles::Profile>>, Vec<Scope>) {
    let live = profiles
        .into_iter()
        .filter(|profile| profile.lease_fence().check_local(now_ms).is_ok())
        .collect::<Vec<_>>();
    let mut scopes = BTreeSet::new();
    for profile in &live {
        let base = crate::storage::scope_for_profile(profile.id.as_str());
        scopes.insert(base.clone());
        for module in crate::modules::registry::ALL {
            if let Ok(scope) = base.module(module.id) {
                scopes.insert(scope);
            }
        }
    }
    (live, scopes.into_iter().collect())
}

/// Retention days from the current config; default when it cannot load.
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
