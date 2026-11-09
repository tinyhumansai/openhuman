//! The tinyflows stores: cron jobs (with their run history and policies), the
//! flow catalog with its drafts, and the flow checkpoints.
//!
//! tinyflows used to own these as SQLite files and JSON drafts under the
//! workspace (`cron/jobs.db`, `agents/<id>/cron/jobs.db`, `flows/flows.db`,
//! `flows/drafts/*.json`, `flows/checkpoints.db`). It no longer has a storage
//! backend of its own, so the host copies what those files hold into the
//! port-backed stores with `tinyflows-import`, which keeps every record under
//! the id it had, writes only what is absent, and reads copies of the files
//! (the originals are never opened for writing).
//!
//! Each store has a marker like the small stores' (see the parent module), so
//! it is copied once: a flow or job deleted afterwards is not resurrected from
//! the untouched legacy file. A failed import leaves no marker and is retried.
//!
//! Where the records land:
//!
//! - the workspace's `cron/jobs.db` and `flows/` go to [`Scope::local`], the
//!   operator's scope;
//! - an embedded agent's `agents/<id>/cron/jobs.db` goes to that agent's scope
//!   ([`crate::storage::scope_for_agent`]), the one its cron calls resolve to.

use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(feature = "flows")]
use std::sync::{LazyLock, Mutex, Weak};

use anyhow::{Context, Result};

use super::{marker_spec, marker_step, Outcome, Report, MARKER_OWNER};
use crate::config::Config;
use crate::storage::documents::Repo;
use crate::storage::import_state;
use crate::storage::{Scope, StorageBackend};

#[cfg(feature = "flows")]
struct CatalogMigrationEntry {
    workspace_dir: PathBuf,
    scope: String,
    backend: Weak<dyn StorageBackend>,
    outcome: std::result::Result<(), String>,
}

#[cfg(feature = "flows")]
static CATALOG_MIGRATIONS: LazyLock<Mutex<Vec<CatalogMigrationEntry>>> =
    LazyLock::new(Default::default);

/// The marker name of the flow catalog (flows, revisions, runs, state,
/// suggestions, drafts) and checkpoints.
pub const FLOWS_DOMAIN: &str = "flows";

/// The marker name of the workspace's own cron store; an agent's is
/// `cron/<agent id>`.
pub const CRON_DOMAIN: &str = "cron";

/// The legacy cron database under `scope_dir` (the workspace, or an agent's
/// directory under it).
pub fn legacy_jobs_db(scope_dir: &Path) -> PathBuf {
    scope_dir.join("cron").join("jobs.db")
}

/// Runs `body` unless the marker for `domain` says it already finished, then
/// records the marker. `body` returns `None` when there is nothing to import
/// (no marker is written then, so a file that appears later is still seen) and
/// otherwise `(found, written, skipped)`. A run that skipped records writes no
/// marker, so the import is retried at the next boot once the operator has
/// repaired them (records already written are counted, not duplicated).
pub(super) fn marked(
    backend: &Arc<dyn StorageBackend>,
    workspace: &Path,
    domain: &str,
    body: impl FnOnce(&tinystoragedrivers::WriteCondition) -> Result<Option<(usize, usize, usize)>>,
) -> Result<Outcome> {
    let step = marker_step(domain, workspace);
    let markers = Repo::on(
        backend,
        &crate::storage::tenant_scope(None),
        "storage::import",
        marker_spec,
    )?;
    if import_state::is_done(&markers, MARKER_OWNER, &step)? {
        return Ok(Outcome::AlreadyDone);
    }
    let claimed = import_state::with_claim(&markers, MARKER_OWNER, &step, |condition| {
        let claimed_markers = markers.conditioned(condition)?;
        if import_state::is_done(&claimed_markers, MARKER_OWNER, &step)? {
            return Ok(Outcome::AlreadyDone);
        }
        let Some((found, written, skipped)) = body(condition)? else {
            return Ok(Outcome::NoSource);
        };
        if skipped > 0 {
            tracing::warn!(
                target: "openhuman::storage_import",
                domain,
                skipped,
                "[storage::import] legacy records were skipped; the import stays open for retry"
            );
            return Ok(Outcome::Incomplete {
                found,
                written,
                skipped,
            });
        }
        import_state::set_done(&claimed_markers, MARKER_OWNER, &step)?;
        Ok(Outcome::Imported { found, written })
    })?;
    Ok(claimed.unwrap_or(Outcome::InProgress))
}

/// Counts the schedule reader's legacy records and logs safe problem details.
fn counts(domain: &str, report: &super::tinyflows_reader::ImportReport) -> (usize, usize, usize) {
    for problem in report.problems() {
        tracing::warn!(
            target: "openhuman::storage_import",
            domain,
            problem = %problem,
            "[storage::import] a legacy record was skipped"
        );
    }
    let found = report
        .kinds()
        .map(|(_, tally)| tally.written + tally.existing + tally.invalid)
        .sum();
    let skipped = report.kinds().map(|(_, tally)| tally.invalid).sum();
    (found, report.total_written(), skipped)
}

#[cfg(feature = "flows")]
#[derive(Debug, Default, Clone, Copy)]
struct ImportTotals {
    found: usize,
    written: usize,
    skipped: usize,
}

#[cfg(feature = "flows")]
impl ImportTotals {
    fn add(&mut self, response: tinyflows_bus::catalog::ImportLegacyBatchResponse) {
        for tally in response.tallies {
            let written = tally.written as usize;
            let existing = tally.existing as usize;
            let invalid = tally.invalid as usize;
            self.found += written + existing + invalid;
            self.written += written;
            self.skipped += invalid;
        }
        for problem in response.problems {
            tracing::warn!(
                target: "openhuman::storage_import",
                domain = FLOWS_DOMAIN,
                kind = ?problem.kind,
                identity = ?problem.identity,
                message = %problem.message,
                "[storage::import] a legacy flow record was skipped"
            );
        }
    }
}

#[cfg(feature = "flows")]
fn flow_sources_present(dir: &Path) -> bool {
    ["flows.db", "checkpoints.db", "checkpoints", "drafts"]
        .iter()
        .any(|name| dir.join(name).exists())
}

/// Copies flow catalog and checkpoint records through TinyFlows' bounded
/// Catalog import contract. Every callback batch completes before the next is
/// read, and the marker is written only after both readers finish cleanly.
///
/// # Errors
///
/// When a legacy file cannot be read, a bus call fails, or the owner store
/// rejects the batch. The legacy files are untouched either way.
#[cfg(feature = "flows")]
pub fn import_flows(
    backend: &Arc<dyn StorageBackend>,
    scope: &Scope,
    config: &Config,
) -> Result<Outcome> {
    let backend_for_migration = Arc::clone(backend);
    let scope_for_migration = scope.clone();
    let config_for_migration = config.clone();
    let migration = crate::storage::block_on_anyhow(async move {
        migrate_catalog_storage(
            &backend_for_migration,
            &config_for_migration,
            &scope_for_migration,
        )
        .await
    });
    record_catalog_migration(
        backend,
        config,
        scope,
        migration.as_ref().map(|_| ()).map_err(ToString::to_string),
    );
    migration?;
    import_flows_after_migration(backend, scope, config)
}

#[cfg(feature = "flows")]
async fn migrate_catalog_storage(
    backend: &Arc<dyn StorageBackend>,
    config: &Config,
    scope: &Scope,
) -> Result<()> {
    let registration = crate::flows::catalog_storage::register_backend_scope(
        config,
        Arc::clone(backend),
        scope.clone(),
    )
    .await
    .map_err(anyhow::Error::msg)?;
    let proxy = crate::modules::tinyflows::catalog_proxy_for_migration(config)
        .await
        .map_err(anyhow::Error::msg)?;
    proxy
        .call::<()>(
            tinyflows_bus::catalog::members::MIGRATE_STORAGE,
            &(registration.storage(),),
        )
        .await
        .context("migrate the TinyFlows catalog storage")
}

#[cfg(feature = "flows")]
fn record_catalog_migration(
    backend: &Arc<dyn StorageBackend>,
    config: &Config,
    scope: &Scope,
    outcome: std::result::Result<(), String>,
) {
    let backend = Arc::downgrade(backend);
    let mut migrations = CATALOG_MIGRATIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    migrations.retain(|entry| entry.backend.strong_count() > 0);
    if let Some(entry) = migrations.iter_mut().find(|entry| {
        entry.workspace_dir == config.workspace_dir
            && entry.scope == scope.as_str()
            && Weak::ptr_eq(&entry.backend, &backend)
    }) {
        entry.outcome = outcome;
    } else {
        migrations.push(CatalogMigrationEntry {
            workspace_dir: config.workspace_dir.clone(),
            scope: scope.as_str().to_owned(),
            backend,
            outcome,
        });
    }
}

#[cfg(feature = "flows")]
fn catalog_migration_ready(
    backend: &Arc<dyn StorageBackend>,
    config: &Config,
    scope: &Scope,
) -> bool {
    let backend = Arc::downgrade(backend);
    CATALOG_MIGRATIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .find(|entry| {
            entry.workspace_dir == config.workspace_dir
                && entry.scope == scope.as_str()
                && Weak::ptr_eq(&entry.backend, &backend)
        })
        .is_some_and(|entry| entry.outcome.is_ok())
}

/// Async on purpose: [`ensure_flow_scope_ready`] runs on the storage blocking
/// bridge, and the synchronous `Repo` helpers would re-enter it ("a future
/// running on the blocking bridge called back into it").
#[cfg(feature = "flows")]
async fn flow_import_marker_done(
    backend: &Arc<dyn StorageBackend>,
    config: &Config,
) -> Result<bool> {
    let scoped = backend
        .for_scope(&crate::storage::tenant_scope(None))
        .context("[storage::import] open the marker scope")?;
    let step = marker_step(FLOWS_DOMAIN, &config.workspace_dir);
    Ok(import_state::done(&scoped, MARKER_OWNER, &step).await?)
}

#[cfg(feature = "flows")]
pub(crate) async fn legacy_flow_import_pending(
    backend: &Arc<dyn StorageBackend>,
    config: &Config,
    scope: &Scope,
) -> Result<bool> {
    if scope != &Scope::local() || !flow_sources_present(&config.workspace_dir.join("flows")) {
        return Ok(false);
    }
    Ok(!flow_import_marker_done(backend, config).await?)
}

/// Make the acting scope safe for workflow calls, retrying a failed Catalog
/// migration before allowing access. A pending local legacy import gates only
/// the local workflow scope; other domains and other storage scopes continue.
#[cfg(feature = "flows")]
pub(crate) async fn ensure_flow_scope_ready(config: &Config) -> std::result::Result<(), String> {
    let scope = crate::storage::current_scope().map_err(|error| error.to_string())?;
    let installed = crate::storage::installed();
    let fallback = installed.is_none();
    let backend = match installed {
        Some(backend) => backend,
        // The workspace-local fallback owns storage preparation and module
        // migrations, but intentionally does not import legacy domain files.
        None => crate::storage::local::backend_for_async(&config.workspace_dir)
            .await
            .map_err(|error| format!("open the fallback storage: {error:#}"))?,
    };
    if !catalog_migration_ready(&backend, config, &scope) {
        let result = migrate_catalog_storage(&backend, config, &scope)
            .await
            .map_err(|error| format!("{error:#}"));
        record_catalog_migration(&backend, config, &scope, result.clone());
        result?;
    }
    if !fallback
        && legacy_flow_import_pending(&backend, config, &scope)
            .await
            .map_err(|error| format!("check the legacy flow import marker: {error:#}"))?
    {
        return Err(
            "legacy flow import is incomplete for this workspace; workflow access is paused until it succeeds"
                .to_owned(),
        );
    }
    Ok(())
}

async fn migrate_schedule_storage(
    backend: &Arc<dyn StorageBackend>,
    config: &Config,
    scope: &Scope,
) -> Result<()> {
    let doc_scope = crate::cron::documents_port::DocScope::on_backend(
        config,
        Arc::clone(backend),
        scope.clone(),
    );
    let _: serde_json::Value = crate::cron::client::store_in(
        config,
        &doc_scope,
        tinyflows_bus::schedule::StoreOperation::Migrate,
    )
    .await
    .context("migrate the TinyFlows Schedule storage")?;
    Ok(())
}

async fn migrate_owner_stores(
    backend: Arc<dyn StorageBackend>,
    config: Config,
) -> (Vec<Report>, bool) {
    // v2 has no global scope registry. Prepare only the owner scopes whose
    // legacy stores this importer can actually discover: the workspace and
    // agent cron directories. The catalog remains workspace-local.
    let local = crate::storage::tenant_scope(None);
    let mut scopes = vec![local.clone()];
    scopes.extend(
        agent_cron_dirs(&config.workspace_dir)
            .into_iter()
            .map(|(agent, _)| crate::storage::scope_for_agent(&agent)),
    );
    scopes.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    scopes.dedup();
    #[cfg(feature = "flows")]
    let mut local_catalog_ready = false;
    #[cfg(not(feature = "flows"))]
    let local_catalog_ready = true;
    let mut reports = Vec::new();

    for scope in scopes {
        // A module's own scope (`.../module:<id>`) holds that module's
        // documents; the catalog and schedule owners never live there, so
        // migrating them in it would leave their markers behind and make a
        // second import change the database.
        if matches!(
            scope.parsed().map(|parsed| parsed.principal),
            Ok(Some(crate::storage::Principal::Module(_)))
        ) {
            continue;
        }
        #[cfg(feature = "flows")]
        {
            let catalog = migrate_catalog_storage(&backend, &config, &scope).await;
            record_catalog_migration(
                &backend,
                &config,
                &scope,
                catalog.as_ref().map(|_| ()).map_err(ToString::to_string),
            );
            if scope == local {
                local_catalog_ready = catalog.is_ok();
            }
            if let Err(error) = catalog {
                reports.push(finish(
                    FLOWS_DOMAIN,
                    Err(error.context(format!("migrate the catalog for scope {}", scope.as_str()))),
                ));
            }
        }

        if let Err(error) = migrate_schedule_storage(&backend, &config, &scope).await {
            reports.push(finish(
                CRON_DOMAIN,
                Err(error.context(format!("migrate the schedule for scope {}", scope.as_str()))),
            ));
        }
    }
    (reports, local_catalog_ready)
}

#[cfg(feature = "flows")]
fn import_flows_after_migration(
    backend: &Arc<dyn StorageBackend>,
    scope: &Scope,
    config: &Config,
) -> Result<Outcome> {
    use tinyflows_bus::catalog::{
        self, ImportLegacyBatchRequest, ImportLegacyBatchResponse, LegacyImportRecord,
    };

    let dir = config.workspace_dir.join("flows");
    marked(backend, &config.workspace_dir, FLOWS_DOMAIN, |condition| {
        if !flow_sources_present(&dir) {
            return Ok(None);
        }
        let dir = dir.clone();
        let backend = import_state::backend_with_condition(Arc::clone(backend), condition.clone());
        let scope = (*scope).clone();
        let config = config.clone();
        let totals = Arc::new(Mutex::new(ImportTotals::default()));
        crate::storage::block_on_anyhow(async move {
            let registration =
                crate::flows::catalog_storage::register_backend_scope(&config, backend, scope)
                    .await
                    .map_err(anyhow::Error::msg)?;
            let storage = registration.storage();
            let proxy = crate::modules::tinyflows::catalog_proxy_for_migration(&config)
                .await
                .map_err(anyhow::Error::msg)?;

            let flow_proxy = proxy.clone();
            let flow_storage = storage.clone();
            let flow_totals = Arc::clone(&totals);
            super::tinyflows_reader::read_legacy_flow_catalog(&dir, move |records| {
                let proxy = flow_proxy.clone();
                let storage = flow_storage.clone();
                let totals = Arc::clone(&flow_totals);
                async move {
                    let request = ImportLegacyBatchRequest { storage, records };
                    let response: ImportLegacyBatchResponse = proxy
                        .call(catalog::members::IMPORT_LEGACY_BATCH, &(request,))
                        .await
                        .context("send a bounded legacy flow batch to TinyFlows")?;
                    totals
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .add(response);
                    Ok(())
                }
            })
            .await
            .context("read the legacy flow catalog")?;

            let checkpoint_proxy = proxy.clone();
            let checkpoint_storage = storage.clone();
            let checkpoint_totals = Arc::clone(&totals);
            super::tinyflows_reader::read_legacy_checkpoint_batches(
                &dir.join("checkpoints.db"),
                &dir.join("checkpoints"),
                move |records: Vec<LegacyImportRecord>| {
                    let proxy = checkpoint_proxy.clone();
                    let storage = checkpoint_storage.clone();
                    let totals = Arc::clone(&checkpoint_totals);
                    async move {
                        let request = ImportLegacyBatchRequest { storage, records };
                        let response: ImportLegacyBatchResponse = proxy
                            .call(catalog::members::IMPORT_LEGACY_BATCH, &(request,))
                            .await
                            .context("send a bounded checkpoint batch to TinyFlows")?;
                        totals
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .add(response);
                        Ok(())
                    }
                },
            )
            .await
            .context("read the legacy flow checkpoints")?;

            let totals = *totals
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            Ok(Some((totals.found, totals.written, totals.skipped)))
        })
    })
}

/// Copies the cron jobs, runs and policies of `scope_dir/cron/jobs.db` into
/// `backend` under `scope`. `label` names the store in its marker: empty for
/// the workspace's own, the agent id for an agent's.
///
/// # Errors
///
/// When the legacy file exists but cannot be read, or the backend fails. The
/// legacy file is untouched either way.
pub fn import_cron(
    backend: &Arc<dyn StorageBackend>,
    scope: &Scope,
    config: &Config,
    scope_dir: &Path,
    label: &str,
) -> Result<Outcome> {
    use crate::cron::client::store_in;
    use crate::cron::documents_port::DocScope;
    use tinyflows_bus::schedule::{ImportedJob, StoreOperation};

    let domain = if label.is_empty() {
        CRON_DOMAIN.to_string()
    } else {
        format!("{CRON_DOMAIN}/{label}")
    };
    let jobs_db = legacy_jobs_db(scope_dir);
    marked(backend, &config.workspace_dir, &domain, |condition| {
        if !jobs_db.is_file() {
            return Ok(None);
        }
        // The legacy file is decoded here (`tinyflows-import`'s reader, bus
        // records only) and replayed through the module's `import_job` /
        // `import_run` store operations, which write only what is absent and
        // file a flow-schedule job under the id its flow derives.
        let backend = import_state::backend_with_condition(Arc::clone(backend), condition.clone());
        let doc_scope = DocScope::on_backend(config, backend, scope.clone());
        let config = config.clone();
        let report = crate::storage::block_on_anyhow({
            let jobs_db = jobs_db.clone();
            async move {
                let legacy = super::tinyflows_reader::read_legacy_cron(&jobs_db)
                    .await
                    .context("read the legacy cron database")?;
                let mut report = legacy.report;
                let mut filed_as = std::collections::HashMap::new();
                for job in legacy.jobs {
                    let source_id = job.id.clone();
                    let imported: ImportedJob =
                        store_in(&config, &doc_scope, StoreOperation::ImportJob { job })
                            .await
                            .context("import a cron job")?;
                    report.wrote("jobs", imported.written);
                    filed_as.insert(source_id, imported.id);
                }
                for mut run in legacy.runs {
                    let Some(id) = filed_as.get(&run.job_id) else {
                        report.invalid(
                            "runs",
                            format!("run {}: job '{}' was not imported", run.id, run.job_id),
                        );
                        continue;
                    };
                    run.job_id = id.clone();
                    let written: bool =
                        store_in(&config, &doc_scope, StoreOperation::ImportRun { run })
                            .await
                            .context("import a cron run")?;
                    report.wrote("runs", written);
                }
                Ok(report)
            }
        })?;
        Ok(Some(counts(&domain, &report)))
    })
}

/// Every agent directory under the workspace that kept its own cron database,
/// as `(agent id, directory)`.
fn agent_cron_dirs(workspace: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(workspace.join("agents")) else {
        return Vec::new();
    };
    let mut dirs: Vec<(String, PathBuf)> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| legacy_jobs_db(&entry.path()).is_file())
        .filter_map(|entry| Some((entry.file_name().into_string().ok()?, entry.path())))
        .collect();
    dirs.sort();
    dirs
}

/// Imports every tinyflows legacy store of `config`'s workspace into
/// `backend`: the flow catalog and checkpoints (only in a build with `flows`),
/// the workspace's cron jobs and each agent's. One [`Report`] per store; a
/// failure in one does not stop the rest. The cron importer needs only the
/// `schedule` reader, so a build without `flows` still keeps its cron jobs.
pub fn run_with(backend: &Arc<dyn StorageBackend>, config: &Config) -> Vec<Report> {
    let backend_for_migration = Arc::clone(backend);
    let config_for_migration = config.clone();
    let (mut reports, _local_catalog_ready) = match crate::storage::block_on_anyhow(async move {
        Ok(migrate_owner_stores(backend_for_migration, config_for_migration).await)
    }) {
        Ok(result) => result,
        Err(error) => (vec![finish(FLOWS_DOMAIN, Err(error))], false),
    };
    #[cfg(feature = "flows")]
    if _local_catalog_ready {
        reports.push(finish(
            FLOWS_DOMAIN,
            import_flows_after_migration(backend, &crate::storage::tenant_scope(None), config),
        ));
    }
    #[cfg(not(feature = "flows"))]
    if config.workspace_dir.join("flows").exists() {
        tracing::warn!(
            target: "openhuman::storage_import",
            "[storage::import] built without `flows`; the legacy flow catalog is not imported"
        );
    }
    reports.push(finish(
        CRON_DOMAIN,
        import_cron(
            backend,
            &crate::storage::tenant_scope(None),
            config,
            &config.workspace_dir,
            "",
        ),
    ));
    for (agent, dir) in agent_cron_dirs(&config.workspace_dir) {
        let scope = crate::storage::scope_for_agent(&agent);
        reports.push(finish(
            CRON_DOMAIN,
            import_cron(backend, &scope, config, &dir, &agent),
        ));
    }
    reports
}

pub(super) fn finish(domain: &'static str, outcome: Result<Outcome>) -> Report {
    match &outcome {
        Ok(Outcome::Imported { found, written }) => tracing::info!(
            target: "openhuman::storage_import",
            domain,
            found,
            written,
            "[storage::import] imported the legacy store; the files were not modified"
        ),
        Ok(Outcome::Incomplete {
            found,
            written,
            skipped,
        }) => tracing::warn!(
            target: "openhuman::storage_import",
            domain,
            found,
            written,
            skipped,
            "[storage::import] some legacy records were skipped; the import remains open for retry"
        ),
        Ok(Outcome::AlreadyDone) => tracing::debug!(
            target: "openhuman::storage_import",
            domain,
            "[storage::import] already imported"
        ),
        Ok(Outcome::InProgress) => tracing::debug!(
            target: "openhuman::storage_import",
            domain,
            "[storage::import] another process is importing; skipped"
        ),
        Ok(Outcome::NoSource) => tracing::debug!(
            target: "openhuman::storage_import",
            domain,
            "[storage::import] no legacy file"
        ),
        Err(error) => tracing::error!(
            target: "openhuman::storage_import",
            domain,
            error = %format!("{error:#}"),
            "[storage::import] import failed; the legacy files are untouched and it is retried on the next boot"
        ),
    }
    Report { domain, outcome }
}

#[cfg(all(test, feature = "flows", feature = "storage-sqlite"))]
#[path = "tinyflows_tests.rs"]
mod tests;

#[cfg(all(test, feature = "storage-sqlite"))]
#[path = "tinyflows_cron_tests.rs"]
mod cron_tests;
