//! One-time, read-only import of the legacy per-workspace stores into the
//! process's storage backend.
//!
//! Before the storage ports, each small domain kept its own SQLite file under
//! the workspace (`approval/approval.db`, `devices/devices.db`,
//! `notifications/notifications.db`, `task_sources/sources.db`). An install
//! that moves to a backend ([`super::install`]) must not start from empty
//! stores, so the host runs this importer once the backend is installed.
//!
//! # Guarantees
//!
//! - **Legacy files are never modified or deleted.** The importer copies the
//!   file (and its `-wal` / `-shm` companions) into a scratch directory and
//!   reads the copy. The legacy readers bring an old schema forward and the
//!   SQLite driver declares its tables when it opens a file; both only ever
//!   touch the copy.
//! - **Two on-disk shapes are read.** The original legacy tables, through the
//!   domain's own reader, and the driver document tables an earlier default
//!   wrote into the same file (`storage::local`). Document-table records win
//!   over legacy rows with the same id: they are the newer copy.
//! - **Idempotent.** Every document is written with `Precondition::Absent`,
//!   so a record that already exists in the backend (a resumed import, or one
//!   the new code has since changed) is never overwritten.
//! - **Runs once.** After a domain's documents are written, the importer is
//!   recorded as done in `tsd.migrations` of the backend's local scope
//!   ([`super::import_state`], step `storage_import/<domain>@<workspace
//!   digest>`). A later boot sees it and does nothing, so a record deleted
//!   since is not resurrected from the untouched legacy file. A marker row an
//!   older build wrote is honoured (it becomes the same entry at boot,
//!   [`super::import_state::migrate_legacy_markers`]). A failed import
//!   records nothing and is retried on the next boot.
//! - **v1 shapes, then the owner's steps.** The legacy readers produce the v1
//!   records the domain used before schema v2; [`upgrade`] runs the domain
//!   owner's step functions over them in memory, so the records are written
//!   to the v2 collections in the v2 shapes with `Precondition::Absent`. A
//!   record the new code already wrote is therefore left alone, the way it
//!   was before schema v2, instead of stopping a migration step.
//!
//! Records land under [`super::scope::tenant_scope`] of no profile
//! (`local`), the one scope every unprofiled reader resolves to
//! ([`super::current_scope`]) — never a scope only some readers use; the legacy files belong to one workspace and carry no tenancy. SaaS
//! processes have no workspace-local legacy files and import nothing.
//!
//! # What is not here
//!
//! Sessions, transcripts, turn states and the run ledger are read by
//! TinyAgents' importer, which owns those formats; see [`sessions`]. The
//! tinyflows stores (cron jobs, the flow catalog and checkpoints) are copied
//! with `tinyflows-import`; see [`tinyflows`].

pub mod sessions;
mod snapshot;
pub mod tinyflows;

use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use sha2::Digest;
use tinystoragedrivers::{CollectionSpec, ErrorKind, Precondition};

use super::documents::Repo;
use super::rusqlite;
use super::{block_on, StorageBackend};
use crate::config::Config;
use rusqlite::Connection;
use serde_json::Value;

pub use snapshot::Snapshot;

/// One legacy row as a document of the new layout.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportDoc {
    /// The collection the store keeps it in.
    pub collection: &'static str,
    /// The document id.
    pub id: String,
    /// The document body.
    pub doc: Value,
}

/// Whether `table` exists on `conn`.
///
/// # Errors
///
/// When the catalog cannot be read.
pub fn table_exists(conn: &Connection, table: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )
}

/// One record to write: a legacy row, or a driver document, in the shape the
/// installed backend expects.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    /// The collection.
    pub collection: String,
    /// The document id.
    pub id: String,
    /// The document body.
    pub doc: serde_json::Value,
}

impl From<ImportDoc> for Record {
    fn from(doc: ImportDoc) -> Self {
        Self {
            collection: doc.collection.to_string(),
            id: doc.id,
            doc: doc.doc,
        }
    }
}

/// The legacy marker collection that recorded finished imports; its name is
/// now the `kind` part of the `tsd.migrations` step
/// ([`super::import_state::step_id`]).
pub const MARKER_KIND: &str = "storage_import";

/// The `tsd.migrations` owner of the host's importers.
pub const MARKER_OWNER: &str = "storage";

/// One legacy store the importer knows how to read.
#[derive(Clone, Copy)]
pub struct Source {
    /// Log and marker name, e.g. `approval`.
    pub domain: &'static str,
    /// The schema owner whose steps carry the imported v1 records to v2.
    pub owner: &'static str,
    /// The legacy database, relative to the workspace directory.
    pub db: &'static str,
    /// The v1 collections the legacy readers write.
    pub collections: fn() -> Vec<CollectionSpec>,
    /// Reads the original legacy tables. Receives a config whose workspace is
    /// the scratch copy (the reader opens and migrates that copy) and the
    /// real config (for keys derived from the real workspace).
    pub read_legacy: fn(scratch: &Config, real: &Config) -> Result<Vec<ImportDoc>>,
    /// Maps a record read from the file's driver document tables to the
    /// shape the installed backend expects, for the few records whose key
    /// depends on the workspace.
    pub rekey: fn(Record, &Config) -> Record,
}

/// How one domain's import ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// No legacy file exists for this workspace.
    NoSource,
    /// A marker says the import already finished.
    AlreadyDone,
    /// Another process holds the import claim and is importing this domain;
    /// this process skips it (the other writes the marker).
    InProgress,
    /// The import ran.
    Imported {
        /// Records found in the legacy file (both shapes).
        found: usize,
        /// Records written; the rest already existed.
        written: usize,
    },
}

/// One domain's result.
#[derive(Debug)]
pub struct Report {
    /// The domain name.
    pub domain: &'static str,
    /// What happened, or why the import failed.
    pub outcome: Result<Outcome>,
}

/// The legacy stores the host imports.
pub fn sources() -> Vec<Source> {
    vec![
        crate::security::approval::import_source(),
        crate::security::devices::import_source(),
        crate::desktop::notifications::import_source(),
        crate::integrations::task_sources::import_source(),
    ]
}

/// Imports every known legacy store into the installed backend, off the
/// async runtime's threads. A no-op when no backend is installed, and in SaaS
/// mode (no workspace-local legacy files).
///
/// Failures are logged and returned in the reports; none stops boot, since
/// the legacy files are untouched and the import is retried next time.
pub async fn run(ctx: &crate::core::runtime::DispatchCtx, config: &Config) -> Vec<Report> {
    let Some(backend) = super::installed() else {
        return Vec::new();
    };
    if crate::core::runtime::mode::is_saas() {
        tracing::debug!(
            target: "openhuman::storage_import",
            "[storage::import] SaaS mode; no workspace-local legacy files to import"
        );
        return Vec::new();
    }
    let config = config.clone();
    let joined = ctx.spawn_blocking(move || run_all(&backend, &config)).await;
    match joined {
        Ok(reports) => reports,
        Err(error) => {
            tracing::error!(
                target: "openhuman::storage_import",
                %error,
                "[storage::import] the import task did not finish"
            );
            Vec::new()
        }
    }
}

/// Every importer, in order, over `backend` for `config`'s workspace; from
/// synchronous code.
pub fn run_all(backend: &Arc<dyn StorageBackend>, config: &Config) -> Vec<Report> {
    let mut reports = run_with(backend, config, &sources());
    reports.extend(tinyflows::run_with(backend, config));
    reports.extend(sessions::run_with(backend, config));
    reports
}

/// [`run`] over an explicit backend and source list, from synchronous code.
pub fn run_with(
    backend: &Arc<dyn StorageBackend>,
    config: &Config,
    sources: &[Source],
) -> Vec<Report> {
    sources
        .iter()
        .map(|source| {
            let outcome = import_one(backend, config, source);
            match &outcome {
                Ok(Outcome::Imported { found, written }) => tracing::info!(
                    target: "openhuman::storage_import",
                    domain = source.domain,
                    found,
                    written,
                    "[storage::import] imported the legacy store; the files were not modified"
                ),
                Ok(Outcome::AlreadyDone) => tracing::debug!(
                    target: "openhuman::storage_import",
                    domain = source.domain,
                    "[storage::import] already imported"
                ),
                Ok(Outcome::InProgress) => tracing::debug!(
                    target: "openhuman::storage_import",
                    domain = source.domain,
                    "[storage::import] another process is importing; skipped"
                ),
                Ok(Outcome::NoSource) => tracing::debug!(
                    target: "openhuman::storage_import",
                    domain = source.domain,
                    "[storage::import] no legacy file"
                ),
                Err(error) => tracing::error!(
                    target: "openhuman::storage_import",
                    domain = source.domain,
                    error = %format!("{error:#}"),
                    "[storage::import] import failed; the legacy files are untouched and it is retried on the next boot"
                ),
            }
            Report {
                domain: source.domain,
                outcome,
            }
        })
        .collect()
}

/// Runs `owner`'s document steps over the v1 `records`, in memory, and returns
/// the v2 records they produce (in input order) with the collection specs to
/// declare. A record in a collection no step reads passes through unchanged.
fn upgrade(owner: &str, records: Vec<Record>) -> Result<(Vec<Record>, Vec<CollectionSpec>)> {
    use tinystoragedrivers::schema::{Item, SourceRecord, StepBody};
    use tinystoragedrivers::Meta;
    let schema = super::schema::owner_schemas()
        .into_iter()
        .find(|schema| schema.owner.as_str() == owner)
        .ok_or_else(|| anyhow!("no storage schema for owner `{owner}`"))?;
    let specs = schema
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Collection { spec, .. } => Some(spec.clone()),
            _ => None,
        })
        .collect();
    let mut upgraded = Vec::with_capacity(records.len());
    let mut by_collection: Vec<(String, Vec<SourceRecord>)> = Vec::new();
    for record in records {
        match by_collection
            .iter_mut()
            .find(|(collection, _)| *collection == record.collection)
        {
            Some((_, batch)) => batch,
            None => {
                by_collection.push((record.collection.clone(), Vec::new()));
                &mut by_collection.last_mut().expect("just pushed").1
            }
        }
        .push(SourceRecord {
            id: record.id,
            body: record.doc,
            meta: Meta::unknown(),
        });
    }
    for (collection, batch) in by_collection {
        let step = schema.steps.iter().find(|step| {
            matches!(step.run, StepBody::Documents(_))
                && step.source.unwrap_or(step.item) == collection
        });
        let Some(StepBody::Documents(run)) = step.map(|step| &step.run) else {
            upgraded.extend(batch.into_iter().map(|r| Record {
                collection: collection.clone(),
                id: r.id,
                doc: r.body,
            }));
            continue;
        };
        for target in run(batch)? {
            upgraded.push(Record {
                collection: target.collection,
                id: target.id,
                doc: target.body,
            });
        }
    }
    Ok((upgraded, specs))
}

/// The marker id for `source` in `workspace`: a backend shared by several
/// workspaces keeps one marker per workspace.
pub(crate) fn marker_id(domain: &str, workspace: &Path) -> String {
    let digest = sha2::Sha256::digest(workspace.as_os_str().as_encoded_bytes());
    format!("{domain}@{}", &hex::encode(digest)[..16])
}

fn marker_spec() -> Vec<CollectionSpec> {
    Vec::new()
}

/// The `tsd.migrations` step of the import of `domain` in `workspace`.
pub(crate) fn marker_step(domain: &str, workspace: &Path) -> String {
    super::import_state::step_id(MARKER_KIND, &marker_id(domain, workspace))
}

fn import_one(
    backend: &Arc<dyn StorageBackend>,
    config: &Config,
    source: &Source,
) -> Result<Outcome> {
    let domain = source.domain;
    // The v2 collections are declared by the write itself (`upgrade` names
    // them); the v1 specs only tell the snapshot which document tables to read.
    let repo = Repo::on(backend, &super::scope::tenant_scope(None), domain, Vec::new)?;
    let markers = Repo::on(
        backend,
        &super::scope::tenant_scope(None),
        "storage::import",
        marker_spec,
    )?;

    let step = marker_step(domain, &config.workspace_dir);
    if super::import_state::is_done(&markers, MARKER_OWNER, &step)? {
        return Ok(Outcome::AlreadyDone);
    }

    let legacy = config.workspace_dir.join(source.db);
    if !legacy.is_file() {
        return Ok(Outcome::NoSource);
    }

    let claimed = super::import_state::with_claim(&markers, MARKER_OWNER, &step, || {
        // Another process may have finished between our check and the claim.
        if super::import_state::is_done(&markers, MARKER_OWNER, &step)? {
            return Ok(Outcome::AlreadyDone);
        }
        let snapshot = Snapshot::of(&legacy, source.db)
            .with_context(|| format!("[{domain}] copy the legacy file for reading"))?;
        let scratch = Config {
            workspace_dir: snapshot.workspace().to_path_buf(),
            ..config.clone()
        };

        // Driver document records first: they are the newer copy of a row whose
        // legacy table was retired, so they win an id clash.
        let mut found: Vec<Record> = snapshot
            .driver_docs(source.collections)
            .with_context(|| format!("[{domain}] read the document tables"))?
            .into_iter()
            .map(|doc| (source.rekey)(doc, config))
            .collect();
        found.extend(
            (source.read_legacy)(&scratch, config)
                .with_context(|| format!("[{domain}] read the legacy tables"))?
                .into_iter()
                .map(Record::from),
        );

        let total = found.len();
        let (found, specs) = upgrade(source.owner, found)
            .with_context(|| format!("[{domain}] bring the legacy records to schema v2"))?;
        let written = repo.run(move |docs| async move {
            for spec in &specs {
                docs.ensure_collection(spec).await?;
            }
            let mut written = 0usize;
            for record in found {
                match docs
                    .put(
                        &record.collection,
                        &record.id,
                        record.doc,
                        Precondition::Absent,
                    )
                    .await
                {
                    Ok(_) => written += 1,
                    Err(error) if error.kind() == ErrorKind::Conflict => {}
                    Err(error) => return Err(error),
                }
            }
            Ok(written)
        })?;

        super::import_state::set_done(&markers, MARKER_OWNER, &step)?;
        Ok(Outcome::Imported {
            found: total,
            written,
        })
    })?;
    Ok(claimed.unwrap_or(Outcome::InProgress))
}

/// Whether the import of `source` is recorded as done (test and diagnostics
/// helper).
pub fn is_done(
    backend: &Arc<dyn StorageBackend>,
    config: &Config,
    source: &Source,
) -> Result<bool> {
    let scoped = backend
        .for_scope(&super::scope::tenant_scope(None))
        .map_err(|error| anyhow!("open the local scope: {error}"))?;
    let step = marker_step(source.domain, &config.workspace_dir);
    Ok(block_on(async move {
        super::import_state::done(&scoped, MARKER_OWNER, &step).await
    })?)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
