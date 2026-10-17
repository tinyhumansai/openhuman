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
//! - **Runs once.** After a domain's documents are written, a marker document
//!   (collection [`MARKER_COLLECTION`], id `<domain>@<workspace digest>`) is
//!   stored in the backend's [`Scope::local`]. A later boot sees the marker
//!   and does nothing, so a record deleted since is not resurrected from the
//!   untouched legacy file. A failed import leaves no marker and is retried
//!   on the next boot.
//!
//! Records land under [`Scope::local`], the scope the operator's own stores
//! read; the legacy files belong to one workspace and carry no tenancy. SaaS
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
use serde_json::json;
use sha2::Digest;
use tinystoragedrivers::{CollectionSpec, ErrorKind, Precondition};

use super::documents::Repo;
use super::rusqlite;
use super::{block_on, Scope, StorageBackend};
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

/// The collection that records finished imports.
pub const MARKER_COLLECTION: &str = "storage_import";

/// Marker document schema version, recorded for diagnostics. An existing
/// marker of any version counts as a finished import; redoing one after an
/// importer change needs an explicit re-import policy (the marker is written
/// with `Precondition::Absent`), which a future bump must introduce.
const MARKER_VERSION: u32 = 1;

/// One legacy store the importer knows how to read.
#[derive(Clone, Copy)]
pub struct Source {
    /// Log and marker name, e.g. `approval`.
    pub domain: &'static str,
    /// The legacy database, relative to the workspace directory.
    pub db: &'static str,
    /// The collections the domain keeps in the backend.
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

/// The marker id for `source` in `workspace`: a backend shared by several
/// workspaces keeps one marker per workspace.
fn marker_id(domain: &str, workspace: &Path) -> String {
    let digest = sha2::Sha256::digest(workspace.as_os_str().as_encoded_bytes());
    format!("{domain}@{}", &hex::encode(digest)[..16])
}

fn marker_spec() -> Vec<CollectionSpec> {
    vec![CollectionSpec::new(MARKER_COLLECTION)]
}

fn import_one(
    backend: &Arc<dyn StorageBackend>,
    config: &Config,
    source: &Source,
) -> Result<Outcome> {
    let domain = source.domain;
    let marker = marker_id(domain, &config.workspace_dir);
    let repo = Repo::on(backend, &Scope::local(), domain, source.collections)?;
    let markers = Repo::on(backend, &Scope::local(), "storage::import", marker_spec)?;

    let marker_id = marker.clone();
    let done = markers
        .run(move |docs| async move { docs.get(MARKER_COLLECTION, &marker_id).await })?
        .is_some();
    if done {
        return Ok(Outcome::AlreadyDone);
    }

    let legacy = config.workspace_dir.join(source.db);
    if !legacy.is_file() {
        return Ok(Outcome::NoSource);
    }

    let claimed =
        super::import_claim::with_claim(&markers, MARKER_COLLECTION, &marker.clone(), || {
            // Another process may have finished between our check and the claim.
            let again = marker.clone();
            let done = markers
                .run(move |docs| async move { docs.get(MARKER_COLLECTION, &again).await })?
                .is_some();
            if done {
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
            let written = repo.run(move |docs| async move {
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

            let body = json!({
                "version": MARKER_VERSION,
                "domain": domain,
                "found": total,
                "written": written,
                "at": chrono::Utc::now().to_rfc3339(),
            });
            markers.run(move |docs| async move {
                match docs
                    .put(MARKER_COLLECTION, &marker, body, Precondition::Absent)
                    .await
                {
                    Ok(_) => Ok(()),
                    // Another process finished first.
                    Err(error) if error.kind() == ErrorKind::Conflict => Ok(()),
                    Err(error) => Err(error),
                }
            })?;
            Ok(Outcome::Imported {
                found: total,
                written,
            })
        })?;
    Ok(claimed.unwrap_or(Outcome::InProgress))
}

/// Whether the marker for `source` exists (test and diagnostics helper).
pub fn is_done(
    backend: &Arc<dyn StorageBackend>,
    config: &Config,
    source: &Source,
) -> Result<bool> {
    let marker = marker_id(source.domain, &config.workspace_dir);
    let scoped = backend
        .for_scope(&Scope::local())
        .map_err(|error| anyhow!("open the local scope: {error}"))?;
    let docs = Arc::clone(scoped.documents());
    let found = block_on(async move {
        docs.ensure_collection(&marker_spec().remove(0)).await?;
        docs.get(MARKER_COLLECTION, &marker).await
    })?;
    Ok(found.is_some())
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
