//! The skill-registry catalog cache on the host's storage ports.
//!
//! Fetched catalogs used to be one JSON file per registry under the cache
//! directory (`~/.openhuman/skill-registry/hermes.json`, tinyskills'
//! `FileCatalogStore`). They are now documents (`tinyskills_catalogs/<registry>`,
//! tinyskills-drivers' `DocumentCatalogStore`) in the process's storage backend
//! under the acting scope, so a SQLite desktop, a MongoDB deployment and a
//! tenant each keep their own cache without a second layout.
//!
//! The cache directory is still the registry's identity: it names the legacy
//! files to import and, when no backend is installed (a unit test, an embedder
//! that skipped the host boot), the workspace whose default backend holds the
//! documents ([`crate::storage::local`]).
//!
//! # Legacy files
//!
//! A catalog with no document is looked up as `<cache dir>/<registry>.json`
//! the first time it is read. The file is parsed, written with
//! `Precondition::Absent` (so a catalog a refresh saved since is never
//! overwritten) and left in place; it is never modified or deleted. Once the
//! document exists it is the only copy read, so a refreshed catalog is not
//! replaced by the older file. A file that is a symlink, oversized, unparseable
//! or in another format is ignored: the catalog is a cache and is refetched.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tinyskills::{is_safe_segment, BoxFuture, CatalogStore, StoreError, StoredCatalog};
use tinyskills_drivers::{DocumentCatalogStore, CATALOG_COLLECTION};
use tinystoragedrivers::{DocumentStore, ErrorKind, Precondition};

const DOMAIN: &str = "skill_registry";
const LOG_PREFIX: &str = "[skill_registry][store]";
/// The largest legacy catalog file read; matches the cap the file store had.
const MAX_LEGACY_BYTES: u64 = 512 * 1024 * 1024;

/// The catalog cache of the process-wide registry.
#[derive(Debug, Clone)]
pub(crate) struct HostCatalogStore {
    /// The cache directory: legacy files live here and, with no backend
    /// installed, so does the workspace default's `storage/` directory.
    dir: PathBuf,
}

impl HostCatalogStore {
    pub(crate) fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The document store for this call's scope.
    async fn documents(&self) -> Result<Arc<dyn DocumentStore>, StoreError> {
        let dir = self.dir.clone();
        tokio::task::spawn_blocking(move || {
            crate::storage::local::open(DOMAIN, &dir).and_then(|opened| opened.scoped())
        })
        .await
        .map_err(|error| StoreError::Io(format!("storage open task failed: {error}")))?
        .map(|scoped| Arc::clone(scoped.documents()))
        .map_err(|error| StoreError::Io(error.to_string()))
    }

    /// Imports `<dir>/<registry>.json` once a read finds no document.
    async fn import_legacy(
        &self,
        registry: &str,
        documents: &Arc<dyn DocumentStore>,
        store: &DocumentCatalogStore,
    ) -> Result<Option<StoredCatalog>, StoreError> {
        let path = self.dir.join(format!("{registry}.json"));
        let Some(catalog) = read_legacy(&path).await else {
            return Ok(None);
        };
        let doc =
            serde_json::to_value(&catalog).map_err(|error| StoreError::Io(error.to_string()))?;
        match documents
            .put(CATALOG_COLLECTION, registry, doc, Precondition::Absent)
            .await
        {
            Ok(_) => {
                tracing::info!(
                    registry,
                    entries = catalog.entries.len(),
                    "{LOG_PREFIX} imported the legacy catalog file"
                );
                Ok(Some(catalog))
            }
            // A refresh saved a catalog between the read and the import: that
            // one is newer than the file, so it wins.
            Err(error)
                if matches!(error.kind(), ErrorKind::Conflict | ErrorKind::AlreadyExists) =>
            {
                store.load(registry).await
            }
            Err(error) => Err(StoreError::Io(error.to_string())),
        }
    }
}

/// The legacy catalog file, or `None` when it is absent or unusable.
async fn read_legacy(path: &Path) -> Option<StoredCatalog> {
    let meta = match tokio::fs::symlink_metadata(path).await {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::debug!(error = %error, "{LOG_PREFIX} legacy catalog file unreadable");
            return None;
        }
    };
    if !meta.is_file() || meta.len() > MAX_LEGACY_BYTES {
        tracing::warn!("{LOG_PREFIX} legacy catalog file is not a plain, bounded file; ignored");
        return None;
    }
    let bytes = tokio::fs::read(path).await.ok()?;
    match serde_json::from_slice::<StoredCatalog>(&bytes) {
        Ok(catalog) if catalog.format == StoredCatalog::FORMAT => Some(catalog),
        Ok(_) => {
            tracing::debug!("{LOG_PREFIX} legacy catalog file is in another format; ignored");
            None
        }
        Err(error) => {
            tracing::warn!(error = %error, "{LOG_PREFIX} legacy catalog file is corrupt; ignored");
            None
        }
    }
}

impl CatalogStore for HostCatalogStore {
    fn load<'a>(
        &'a self,
        registry: &'a str,
    ) -> BoxFuture<'a, Result<Option<StoredCatalog>, StoreError>> {
        Box::pin(async move {
            if !is_safe_segment(registry) {
                return Err(StoreError::InvalidId(registry.to_owned()));
            }
            let documents = self.documents().await?;
            let store = DocumentCatalogStore::new(Arc::clone(&documents));
            if let Some(found) = store.load(registry).await? {
                return Ok(Some(found));
            }
            self.import_legacy(registry, &documents, &store).await
        })
    }

    fn save<'a>(
        &'a self,
        registry: &'a str,
        catalog: &'a StoredCatalog,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            let documents = self.documents().await?;
            DocumentCatalogStore::new(documents)
                .save(registry, catalog)
                .await
        })
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
