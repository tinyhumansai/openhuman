//! The cost ledger on the `tinystoragedrivers` document port.
//!
//! The only ledger: `CostStorage` in `tracker.rs` opens it per call, so
//! records live under the acting agent's storage scope (`local` on a
//! single-user host) in the installed backend, or the workspace default when
//! none is installed ([`crate::storage::local`]). A legacy `state/costs.jsonl`
//! is imported once.
//!
//! # Layout
//!
//! | Collection | Document id | Holds |
//! | --- | --- | --- |
//! | `cost.records` | `Key::of([record id])` | one [`CostRecord`], its usage timestamp carried once as `ts_ms` (epoch ms, [`super::storage_schema`]) |
//!
//! The JSONL ledger is append-only, so a record is written once
//! (`Precondition::Absent`) and never edited. Readers get the records oldest
//! first, the order the file kept them in.

use anyhow::{Context, Result};
use tinystoragedrivers::{CollectionSpec, Precondition, Query, Sort};

use super::storage_schema::{from_body, spec, to_body, RECORDS};
use super::types::CostRecord;
use crate::storage::documents::Repo;
use crate::storage::schema_support::key_of;
use crate::storage::DocumentStoreExt;

const DOMAIN: &str = "cost::tracker";

fn collections() -> Vec<CollectionSpec> {
    vec![spec()]
}

/// The document store for this call: the installed backend, else the
/// workspace default, in the acting scope.
pub(super) fn current(workspace_dir: &std::path::Path) -> Result<CostDocs> {
    #[cfg(test)]
    if let Some(docs) = TEST_OVERRIDE.with(|slot| slot.borrow().clone()) {
        return Ok(docs);
    }
    Ok(CostDocs(Repo::current_for(
        workspace_dir,
        DOMAIN,
        collections,
    )?))
}

/// Runs `f` with `docs` standing in for the installed backend, on this thread
/// only, so tests exercise the dispatch without the process-wide slot.
#[cfg(test)]
pub(super) fn with_override<T>(docs: CostDocs, f: impl FnOnce() -> T) -> T {
    TEST_OVERRIDE.with(|slot| *slot.borrow_mut() = Some(docs));
    let out = f();
    TEST_OVERRIDE.with(|slot| *slot.borrow_mut() = None);
    out
}

#[cfg(test)]
thread_local! {
    static TEST_OVERRIDE: std::cell::RefCell<Option<CostDocs>> = const { std::cell::RefCell::new(None) };
}

/// The cost ledger over one scoped document handle.
#[derive(Clone)]
pub(super) struct CostDocs(Repo);

impl CostDocs {
    #[cfg(test)]
    pub(super) fn over(scoped: &crate::storage::ScopedStorage) -> Self {
        Self(Repo::over(scoped, DOMAIN, collections))
    }

    /// Appends one record.
    pub(super) fn add(&self, record: &CostRecord) -> Result<()> {
        let doc = to_body(record).context("serialize cost record")?;
        let id = key_of(&record.id)?;
        log::trace!("[cost::tracker] document add id={id}");
        self.0.run(|docs| async move {
            docs.put(RECORDS, &id, doc, Precondition::Absent)
                .await
                .map(|_| ())
        })
    }

    /// Imports a legacy `state/costs.jsonl`, then renames it to
    /// `costs.jsonl.migrated` so it is not read twice. Returns how many
    /// records were created. A record whose id is already stored (by an
    /// earlier attempt, another core importing at the same time, or a repeat
    /// line in the file) is skipped, so the import is idempotent. Malformed
    /// lines are skipped, as readers skip them.
    ///
    /// # Errors
    ///
    /// A storage error, or a failed rename: the ledger then stays in place and
    /// the import is not recorded as done, so the next call retries it (and
    /// creates nothing twice).
    pub(super) fn import_legacy(&self, path: &std::path::Path) -> Result<usize> {
        let raw = match std::fs::read(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("read legacy cost ledger {}", path.display()))
            }
        };
        let mut imported = 0;
        for line in raw.split(|byte| *byte == b'\n') {
            let Ok(line) = std::str::from_utf8(line) else {
                log::warn!("[cost::tracker] legacy import: skipping a line that is not UTF-8");
                continue;
            };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<CostRecord>(line) {
                Ok(record) => {
                    if self.add_if_absent(&record)? {
                        imported += 1;
                    }
                }
                Err(error) => log::warn!("[cost::tracker] legacy import: skipping line: {error}"),
            }
        }
        set_aside(path)?;
        log::debug!("[cost::tracker] legacy import: imported={imported}");
        Ok(imported)
    }

    /// [`Self::import_legacy`] once per process for this backend instance,
    /// scope and ledger path; a failed import is retried by the next call.
    pub(super) fn import_legacy_once(&self, path: &std::path::Path) -> Result<()> {
        let key = path.display().to_string();
        self.0
            .once_per_backend(&key, || self.import_legacy(path).map(|_| ()))
    }

    /// Writes `record` unless its id is already stored; `false` when it was.
    fn add_if_absent(&self, record: &CostRecord) -> Result<bool> {
        let doc = to_body(record).context("serialize cost record")?;
        let id = key_of(&record.id)?;
        self.0.run(|docs| async move {
            match docs.put(RECORDS, &id, doc, Precondition::Absent).await {
                Ok(_) => Ok(true),
                Err(error) if error.kind() == tinystoragedrivers::ErrorKind::Conflict => Ok(false),
                Err(error) => Err(error),
            }
        })
    }

    /// Every record, oldest first. Documents that no longer parse are skipped
    /// with a warning, as malformed ledger lines are.
    pub(super) fn all(&self) -> Result<Vec<CostRecord>> {
        let stored = self.0.run(|docs| async move {
            docs.query_all(
                RECORDS,
                &Query::all().sort(Sort::asc("ts_ms")).sort(Sort::asc("_id")),
            )
            .await
        })?;
        Ok(stored
            .into_iter()
            .filter_map(|item| match from_body(item.doc) {
                Ok(record) => Some(record),
                Err(error) => {
                    log::warn!(
                        "[cost::tracker] skipping malformed cost document id={} error={error}",
                        item.id
                    );
                    None
                }
            })
            .collect())
    }
}

/// Renames the imported ledger to `costs.jsonl.migrated`, or the first free
/// `costs.jsonl.migrated.<n>` so an earlier set-aside file is never replaced.
/// A ledger already gone was set aside by another core importing at the same
/// time, which is success.
fn set_aside(path: &std::path::Path) -> Result<()> {
    let base = path.with_extension("jsonl.migrated");
    let mut target = base.clone();
    let mut n = 0;
    while target.exists() {
        n += 1;
        target = std::path::PathBuf::from(format!("{}.{n}", base.display()));
    }
    match std::fs::rename(path, &target) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !path.exists() => {
            log::debug!("[cost::tracker] legacy import: ledger already set aside");
            Ok(())
        }
        Err(error) => Err(error).with_context(|| {
            format!(
                "set aside legacy cost ledger {} as {}",
                path.display(),
                target.display()
            )
        }),
    }
}

#[cfg(test)]
#[path = "tracker_documents_tests.rs"]
mod tests;
