//! Versioned artifact records on the `tinystoragedrivers` document port.
//!
//! [`DocumentArtifactStore`] is OpenHuman's `tiny-artifacts`
//! [`ArtifactStore`]: each record lives in the installed storage backend (or
//! the workspace default when none is installed), under the acting agent's
//! scope, like the rest of the artifact domain (`store_documents.rs`). So a
//! shared database shares the histories, and a core on another node or after
//! a cloud restart reads the same versions.
//!
//! | Collection | Document id | Holds |
//! | --- | --- | --- |
//! | `artifacts.versions` | `Key::of([artifact id])` | the whole `ArtifactRecord` with snake_case keys and `*_ms` times ([`super::storage_schema`]) |
//!
//! Only the records move. A file version's bytes stay in a file
//! (`ArtifactVersion::file`), as the legacy card's do.
//!
//! The first call of a process imports the records an earlier build kept as
//! `<workspace>/artifacts/versioned/<id>.json` (once per backend instance,
//! scope and workspace). Only absent records are written, so the import never
//! overwrites a newer one and is safe to repeat; deleting an artifact removes
//! its legacy file first, so a restart does not bring it back.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;
use tiny_artifacts::{ArtifactRecord, ArtifactStore};
use tinystoragedrivers::{CollectionSpec, Precondition, Query, Sort};

use super::storage_schema::{version_body, version_record, VERSIONS};
use crate::storage::documents::Repo;
use crate::storage::schema_support::key_of;
use crate::storage::DocumentStoreExt;

const DOMAIN: &str = "artifacts::versions";

fn collections() -> Vec<CollectionSpec> {
    vec![CollectionSpec::new(VERSIONS)]
}

/// The versioned records of one workspace, in the configured storage backend.
#[derive(Clone, Debug)]
pub struct DocumentArtifactStore {
    workspace_dir: PathBuf,
}

impl DocumentArtifactStore {
    /// The store for `workspace_dir`'s records.
    pub fn new(workspace_dir: impl Into<PathBuf>) -> Self {
        Self {
            workspace_dir: workspace_dir.into(),
        }
    }

    /// Where an earlier build kept the records as JSON files.
    fn legacy_dir(&self) -> PathBuf {
        super::tiny_store::legacy_versioned_dir(&self.workspace_dir)
    }

    /// The repo for this call (resolved here, in the caller's context, for its
    /// scope), after the one-time legacy import.
    async fn repo(&self) -> Result<Repo> {
        let repo = Repo::current_for(&self.workspace_dir, DOMAIN, collections)?;
        let legacy = self.legacy_dir();
        on_blocking(repo.clone(), move |repo| {
            let key = legacy.display().to_string();
            repo.once_per_backend(&key, || import_legacy(repo, &legacy).map(|_| ()))
        })
        .await?;
        Ok(repo)
    }
}

#[async_trait]
impl ArtifactStore for DocumentArtifactStore {
    async fn list(&self) -> Result<Vec<ArtifactRecord>> {
        let repo = self.repo().await?;
        let stored = on_blocking(repo, |repo| {
            repo.run(|docs| async move {
                docs.query_all(
                    VERSIONS,
                    &Query::all()
                        .sort(Sort::desc("updated_ms"))
                        .sort(Sort::asc("_id")),
                )
                .await
            })
        })
        .await?;
        Ok(stored
            .into_iter()
            .filter_map(|item| match record_of(item.doc) {
                Ok(record) => Some(record),
                Err(err) => {
                    tracing::warn!(
                        target: "tiny-artifacts",
                        id = %item.id,
                        err = %err,
                        "[artifacts] skipping an unreadable versioned record"
                    );
                    None
                }
            })
            .collect())
    }

    async fn get(&self, id: &str) -> Result<Option<ArtifactRecord>> {
        let repo = self.repo().await?;
        let id = key_of(id)?;
        let stored = on_blocking(repo, move |repo| {
            repo.run(|docs| async move { docs.get(VERSIONS, &id).await })
        })
        .await?;
        stored.map(|stored| record_of(stored.doc)).transpose()
    }

    async fn upsert(&self, artifact: &ArtifactRecord) -> Result<()> {
        let repo = self.repo().await?;
        let id = key_of(&artifact.id)?;
        let doc = doc_of(artifact)?;
        on_blocking(repo, move |repo| {
            repo.run(|docs| async move {
                docs.put(VERSIONS, &id, doc, Precondition::None)
                    .await
                    .map(|_| ())
            })
        })
        .await
    }

    async fn delete(&self, id: &str) -> Result<bool> {
        // The legacy file goes first: it is what a later import would bring
        // back.
        if !id.is_empty() && !id.contains(['/', '\\']) && !id.starts_with('.') {
            let legacy = self.legacy_dir().join(format!("{id}.json"));
            match tokio::fs::remove_file(&legacy).await {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(err).with_context(|| format!("remove {}", legacy.display()))
                }
            }
        }
        let repo = self.repo().await?;
        let id = key_of(id)?;
        on_blocking(repo, move |repo| {
            repo.run(|docs| async move { docs.delete(VERSIONS, &id, Precondition::None).await })
        })
        .await
    }
}

fn doc_of(record: &ArtifactRecord) -> Result<Value> {
    Ok(version_body(
        &serde_json::to_value(record).context("serialize artifact record")?,
    ))
}

fn record_of(doc: Value) -> Result<ArtifactRecord> {
    serde_json::from_value(version_record(&doc)).context("parse artifact record")
}

/// Imports the legacy `<id>.json` records in `dir` that the store does not
/// have; returns how many it created. A missing folder is nothing to import
/// and an unreadable record is skipped; any other read error fails the import,
/// so it is retried.
fn import_legacy(repo: &Repo, dir: &Path) -> Result<usize> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err).with_context(|| format!("read {}", dir.display())),
    };
    let mut imported = 0;
    for entry in entries {
        let path = entry
            .with_context(|| format!("read {}", dir.display()))?
            .path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let raw =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let Ok(record) = serde_json::from_str::<ArtifactRecord>(&raw) else {
            tracing::warn!(
                target: "tiny-artifacts",
                file = %path.display(),
                "[artifacts] legacy import: skipping an unreadable record"
            );
            continue;
        };
        // A record is kept under its own id; a copy under another name is not
        // the record.
        if path.file_stem().and_then(|s| s.to_str()) != Some(record.id.as_str()) {
            continue;
        }
        let id = key_of(&record.id)?;
        let doc = doc_of(&record)?;
        let created = repo.run(|docs| async move {
            match docs.put(VERSIONS, &id, doc, Precondition::Absent).await {
                Ok(_) => Ok(true),
                Err(err) if err.kind() == tinystoragedrivers::ErrorKind::Conflict => Ok(false),
                Err(err) => Err(err),
            }
        })?;
        if created {
            imported += 1;
        }
    }
    tracing::debug!(
        target: "tiny-artifacts",
        imported,
        "[artifacts] versioned records: legacy import done"
    );
    Ok(imported)
}

/// Runs a document-store call off the async worker (the port is reached
/// through a blocking bridge).
async fn on_blocking<T: Send + 'static>(
    repo: Repo,
    f: impl FnOnce(&Repo) -> Result<T> + Send + 'static,
) -> Result<T> {
    crate::core::runtime::DispatchCtx::ambient()
        .spawn_blocking(move || f(&repo))
        .await
        .map_err(|e| anyhow::anyhow!("[artifacts] storage task failed: {e}"))?
}

#[cfg(test)]
#[path = "version_documents_tests.rs"]
mod tests;
