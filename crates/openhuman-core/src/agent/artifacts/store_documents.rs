//! Artifact records on the `tinystoragedrivers` document port.
//!
//! The only record store (`<workspace>/artifacts/<id>/meta.json` and
//! `args.json` are legacy, imported once): `store.rs` opens it per call, so the
//! records live under the acting agent's storage scope (`local` on a
//! single-user host) in the installed backend, or the workspace default when
//! none is installed ([`crate::storage::local`]).
//!
//! # Layout
//!
//! | Collection | Document id | Holds |
//! | --- | --- | --- |
//! | `artifacts.records` | `Key::of([artifact id])` | the [`ArtifactMeta`] fields with `created_ms` in place of `created_at` |
//! | `artifacts.args` | `Key::of([artifact id])` | the producer-tool arguments (the `args.json` sidecar), unwrapped |
//!
//! The shapes and the v1 -> v2 steps are in [`super::storage_schema`].
//!
//! # Content files
//!
//! Only the records move. An artifact's bytes are a file in the visible files
//! folder (`ArtifactMeta::file`), which the user opens, edits and deletes
//! outside OpenHuman; the port has no blob store for that, so the file stays
//! where it is on every backend and the record names it. A shared database
//! therefore shares the list, the status and the regenerate arguments, not the
//! bytes.
//!
//! The first document-mode call of a process imports the workspace's legacy
//! `meta.json` / `args.json` files into the acting backend and scope
//! ([`Docs::import_legacy`], once per backend instance, scope and workspace).
//! The import only creates records that are absent (`Precondition::Absent`),
//! so it never overwrites a record written since and is safe to repeat; the
//! files stay in place, and deleting an artifact in document mode removes its
//! legacy directory too, so a restart does not bring it back.

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;
use tinystoragedrivers::{CollectionSpec, Filter, Precondition, Query, Sort};

use super::storage_schema::{
    args_body, args_of_body, meta_from_body, meta_to_body, records_spec, ARGS, RECORDS as ARTIFACTS,
};
use super::types::ArtifactMeta;
use crate::storage::documents::Repo;
use crate::storage::schema_support::key_of;
use crate::storage::DocumentStoreExt;

const DOMAIN: &str = "artifacts::store";

fn collections() -> Vec<CollectionSpec> {
    vec![records_spec(), CollectionSpec::new(ARGS)]
}

/// The document store for this call: the installed backend, else the
/// workspace default, in the acting scope.
pub(super) fn current(workspace_dir: &Path) -> Result<Docs> {
    #[cfg(test)]
    if let Some(docs) = TEST_OVERRIDE.with(|slot| slot.borrow().clone()) {
        return Ok(docs);
    }
    Ok(Docs(Repo::current_for(workspace_dir, DOMAIN, collections)?))
}

/// Runs `f` with `docs` standing in for the installed backend, on this thread
/// only, so tests exercise the dispatch without the process-wide slot.
#[cfg(test)]
pub(super) fn with_override<T>(docs: Docs, f: impl FnOnce() -> T) -> T {
    TEST_OVERRIDE.with(|slot| *slot.borrow_mut() = Some(docs));
    let out = f();
    TEST_OVERRIDE.with(|slot| *slot.borrow_mut() = None);
    out
}

#[cfg(test)]
thread_local! {
    static TEST_OVERRIDE: std::cell::RefCell<Option<Docs>> = const { std::cell::RefCell::new(None) };
}

/// The artifact records over one scoped document handle.
#[derive(Clone)]
pub(super) struct Docs(Repo);

impl Docs {
    #[cfg(test)]
    pub(super) fn over(scoped: &crate::storage::ScopedStorage) -> Self {
        Self(Repo::over(scoped, DOMAIN, collections))
    }

    pub(super) fn put_meta(&self, meta: &ArtifactMeta) -> Result<()> {
        let doc = meta_to_body(meta).context("serialize artifact meta")?;
        let id = key_of(&meta.id)?;
        self.0.run(|docs| async move {
            docs.put(ARTIFACTS, &id, doc, Precondition::None)
                .await
                .map(|_| ())
        })
    }

    /// Writes the record only when none exists, atomically (an import must
    /// not overwrite a record created since, by this core or another).
    /// `false` when one was already there.
    fn put_meta_if_absent(&self, meta: &ArtifactMeta) -> Result<bool> {
        let doc = meta_to_body(meta).context("serialize artifact meta")?;
        let id = key_of(&meta.id)?;
        self.0
            .run(|docs| async move { put_absent(&docs, ARTIFACTS, &id, doc).await })
    }

    /// The arguments' counterpart of [`Self::put_meta_if_absent`].
    fn put_args_if_absent(&self, id: &str, args: &Value) -> Result<bool> {
        let id = key_of(id)?;
        let doc = args_body(args);
        self.0
            .run(|docs| async move { put_absent(&docs, ARGS, &id, doc).await })
    }

    pub(super) fn get_meta(&self, id: &str) -> Result<Option<ArtifactMeta>> {
        let id = key_of(id)?;
        let stored = self
            .0
            .run(|docs| async move { docs.get(ARTIFACTS, &id).await })?;
        stored
            .map(|stored| meta_from_body(stored.doc).context("parse artifact meta"))
            .transpose()
    }

    /// Every record, newest first. A document that no longer parses is skipped
    /// with a warning, as a corrupt `meta.json` is.
    pub(super) fn list_meta(&self) -> Result<Vec<ArtifactMeta>> {
        self.query_meta(Query::all())
    }

    /// The records of one thread, newest first, read through the `by_thread`
    /// index rather than by listing the workspace.
    pub(super) fn list_meta_for_thread(&self, thread_id: &str) -> Result<Vec<ArtifactMeta>> {
        self.query_meta(Query::filter(Filter::eq("thread_id", thread_id)))
    }

    fn query_meta(&self, query: Query) -> Result<Vec<ArtifactMeta>> {
        let query = query.sort(Sort::desc("created_ms")).sort(Sort::asc("_id"));
        let stored = self
            .0
            .run(|docs| async move { docs.query_all(ARTIFACTS, &query).await })?;
        Ok(stored
            .into_iter()
            .filter_map(|item| match meta_from_body(item.doc) {
                Ok(meta) => Some(meta),
                Err(error) => {
                    log::warn!(
                        "[artifacts] skipping artifact document id={} error={error}",
                        item.id
                    );
                    None
                }
            })
            .collect())
    }

    pub(super) fn put_args(&self, id: &str, args: &Value) -> Result<()> {
        let id = key_of(id)?;
        let doc = args_body(args);
        self.0.run(|docs| async move {
            docs.put(ARGS, &id, doc, Precondition::None)
                .await
                .map(|_| ())
        })
    }

    pub(super) fn get_args(&self, id: &str) -> Result<Option<Value>> {
        let id = key_of(id)?;
        let stored = self
            .0
            .run(|docs| async move { docs.get(ARGS, &id).await })?;
        Ok(stored.map(|stored| args_of_body(&stored.doc)))
    }

    /// Removes the record and its arguments; `false` when there was no record.
    /// The record goes first: if the arguments delete then fails, a retry
    /// finds no record and only an orphaned arguments document is left, never
    /// a listed artifact that lost its regenerate arguments.
    pub(super) fn delete(&self, id: &str) -> Result<bool> {
        let id = key_of(id)?;
        self.0.run(|docs| async move {
            let existed = docs.delete(ARTIFACTS, &id, Precondition::None).await?;
            docs.delete(ARGS, &id, Precondition::None).await?;
            Ok(existed)
        })
    }

    /// Imports the workspace's legacy `meta.json` / `args.json` files and
    /// returns how many records it created. Only absent records are written,
    /// so a repeat is harmless, and an interrupted import (meta in, args not)
    /// is completed by the next one. A missing folder or a folder without
    /// `meta.json` is nothing to import and a corrupt `meta.json` is skipped,
    /// but any other read error fails the import, so it is retried rather than
    /// recorded as done.
    pub(super) fn import_legacy(&self, artifacts_dir: &Path) -> Result<usize> {
        let entries = match std::fs::read_dir(artifacts_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => {
                return Err(error).with_context(|| format!("read {}", artifacts_dir.display()))
            }
        };
        let mut imported = 0;
        for entry in entries {
            let dir = entry
                .with_context(|| format!("read {}", artifacts_dir.display()))?
                .path();
            if !dir.is_dir() {
                continue;
            }
            let Some(raw) = read_optional(&dir.join("meta.json"))? else {
                continue;
            };
            let Ok(meta) = serde_json::from_str::<ArtifactMeta>(&raw) else {
                log::warn!(
                    "[artifacts] legacy import: skipping corrupt meta.json in {}",
                    dir.display()
                );
                continue;
            };
            // The file store keeps each record in `artifacts/<id>/`; a copy
            // under another name is not the record, so the id is unique and
            // the winner does not depend on directory order.
            if dir.file_name().and_then(|name| name.to_str()) != Some(meta.id.as_str()) {
                log::warn!(
                    "[artifacts] legacy import: skipping meta.json whose id does not match its folder {}",
                    dir.display()
                );
                continue;
            }
            if self.put_meta_if_absent(&meta)? {
                imported += 1;
            }
            let args = read_optional(&dir.join("args.json"))?
                .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
            if let Some(args) = args {
                self.put_args_if_absent(&meta.id, &args)?;
            }
        }
        log::debug!("[artifacts] legacy import: imported={imported}");
        Ok(imported)
    }
}

/// Puts `doc` only when `id` is absent; `false` when it was already there.
async fn put_absent(
    docs: &std::sync::Arc<dyn crate::storage::DocumentStore>,
    collection: &str,
    id: &str,
    doc: Value,
) -> Result<bool, crate::storage::StorageError> {
    match docs.put(collection, id, doc, Precondition::Absent).await {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == tinystoragedrivers::ErrorKind::Conflict => Ok(false),
        Err(error) => Err(error),
    }
}

/// The file's contents, `None` when it does not exist.
fn read_optional(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(raw) => Ok(Some(raw)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

/// Runs a document-store call off the async worker (the port is reached
/// through a blocking bridge).
pub(super) async fn on_docs<T: Send + 'static>(
    docs: Docs,
    f: impl FnOnce(Docs) -> anyhow::Result<T> + Send + 'static,
) -> Result<T, String> {
    crate::core::runtime::DispatchCtx::ambient()
        .spawn_blocking(move || f(docs))
        .await
        .map_err(|e| format!("[artifacts] storage task failed: {e}"))?
        .map_err(|e| format!("[artifacts] storage: {e:#}"))
}

/// The document store for this call. The first call of a process for this backend instance, scope and workspace
/// imports the workspace's legacy `meta.json` / `args.json` files (recorded
/// only once the import succeeded, so a failed one is retried).
pub(super) async fn documents(workspace_dir: &Path) -> Result<Docs, String> {
    let docs = current(workspace_dir).map_err(|e| format!("[artifacts] storage: {e:#}"))?;
    let legacy = workspace_dir.join("artifacts");
    on_docs(docs.clone(), move |docs| {
        let key = legacy.display().to_string();
        docs.0
            .once_per_backend(&key, || docs.import_legacy(&legacy).map(|_| ()))
    })
    .await?;
    Ok(docs)
}

/// Deletes artifact `id` in document mode. The legacy directory
/// (`artifact_dir`) goes first: it is what a later import would bring back,
/// so if anything after it fails (or the process dies) the artifact is still
/// listed and a retry finishes the job, rather than a gone record coming back
/// from a leftover `meta.json`. Not found when there was no record, as the
/// file store reports.
pub(super) async fn delete_record(docs: Docs, artifact_dir: &Path, id: &str) -> Result<(), String> {
    match tokio::fs::remove_dir_all(artifact_dir).await {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(format!(
                "[artifacts] failed to delete legacy record for id={id}: {e}"
            ))
        }
    }
    let owned = id.to_string();
    let existed = on_docs(docs, move |docs| docs.delete(&owned)).await?;
    if !existed {
        return Err(format!(
            "[artifacts] failed to delete artifact id={id}: not found"
        ));
    }
    log::debug!("[artifacts] delete_artifact: deleted record id={id}");
    Ok(())
}

/// Sorts newest first, applies the thread filter and then the page.
pub(super) fn page_of(
    mut all: Vec<ArtifactMeta>,
    offset: usize,
    limit: usize,
    thread_id: Option<&str>,
) -> (Vec<ArtifactMeta>, usize) {
    // Sort descending by created_at (newest first)
    all.sort_by_key(|item| std::cmp::Reverse(item.created_at));

    // Apply thread filter BEFORE pagination so `total` reflects the
    // per-thread count the UI surfaces, and so a small page doesn't get
    // silently emptied by filtering after the slice (#3226).
    if let Some(tid) = thread_id {
        all.retain(|m| m.thread_id.as_deref() == Some(tid));
    }

    let total = all.len();
    let page = all.into_iter().skip(offset).take(limit).collect::<Vec<_>>();

    log::debug!(
        "[artifacts] list_artifacts: total={total} returning {} items",
        page.len()
    );
    (page, total)
}

#[cfg(test)]
#[path = "store_documents_tests.rs"]
mod tests;
