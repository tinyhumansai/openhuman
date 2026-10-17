//! [`ProfileRegistry`]: which profiles are provisioned.
//!
//! With a storage backend installed, every node of a deployment reads one
//! registry: collection [`REGISTRY_COLLECTION`] of the shared
//! [`CLUSTER_SCOPE`](crate::storage::lease::CLUSTER_SCOPE), one
//! document per profile (`{profile_id, layout_version}`, the creation time in
//! the record's metadata; [`super::storage_schema`]). Without one it is the
//! `profile.toml` file beside each profile's state
//! (`<root>/users/<id>/profile.toml`), as before.
//!
//! The registry records existence and creation time only. A profile's config
//! is derived (`layout::profile_config`), never stored, and its state stays
//! under `<root>/users/<id>/` either way.

use std::path::PathBuf;
use std::sync::Arc;

use tinystoragedrivers::{
    CollectionSpec, DocumentStoreExt, ErrorKind, Precondition, Query, Scope, ScopedStorage,
    Versioned, WriteOptions,
};

use super::layout::{self, ProfileLayout};
use super::storage_schema::migrate_cluster;
pub use super::storage_schema::REGISTRY_COLLECTION;
use super::types::{ProfileId, ProfileMeta};
use crate::storage::lease::CLUSTER_SCOPE;
use crate::storage::schema_support::key_of;
use crate::storage::{DocumentStore, StorageBackend};

/// The profile a registry document holds: the body's fields, and the creation
/// time (Unix seconds) from the record's metadata.
fn meta_of(stored: &Versioned<serde_json::Value>) -> Result<ProfileMeta, serde_json::Error> {
    let mut doc = stored.doc.clone();
    doc["created_at"] = (stored.meta.created_ms.max(0) / 1000).into();
    serde_json::from_value(doc)
}

/// The body a profile is stored as: its fields without the creation time.
fn body_of(meta: &ProfileMeta) -> Result<serde_json::Value, serde_json::Error> {
    let mut doc = serde_json::to_value(meta)?;
    if let Some(fields) = doc.as_object_mut() {
        fields.remove("created_at");
    }
    Ok(doc)
}

/// Where provisioned profiles are recorded. See the module docs.
pub enum ProfileRegistry {
    /// One document per profile in the shared backend.
    Documents {
        docs: Arc<dyn DocumentStore>,
        scoped: ScopedStorage,
        declared: tokio::sync::OnceCell<()>,
    },
    /// `profile.toml` under each profile's directory below `root`.
    Files { root: PathBuf },
}

impl std::fmt::Debug for ProfileRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Documents { .. } => f.write_str("ProfileRegistry::Documents"),
            Self::Files { root } => f
                .debug_struct("ProfileRegistry::Files")
                .field("root", root)
                .finish(),
        }
    }
}

impl ProfileRegistry {
    /// `profile.toml` files under the SaaS `root`.
    pub fn files(root: impl Into<PathBuf>) -> Self {
        Self::Files { root: root.into() }
    }

    /// Documents in `backend`'s cluster scope.
    ///
    /// # Errors
    ///
    /// When the backend refuses the scope.
    pub fn cluster(backend: &dyn StorageBackend) -> Result<Self, String> {
        let scope = Scope::new(CLUSTER_SCOPE).map_err(|e| e.to_string())?;
        let scoped = backend.for_scope(&scope).map_err(|e| e.to_string())?;
        Ok(Self::Documents {
            docs: Arc::clone(scoped.documents()),
            scoped,
            declared: tokio::sync::OnceCell::new(),
        })
    }

    /// Whether this registry is the shared backend.
    pub fn is_shared(&self) -> bool {
        matches!(self, Self::Documents { .. })
    }

    async fn docs(&self) -> Result<Option<&Arc<dyn DocumentStore>>, String> {
        let Self::Documents {
            docs,
            scoped,
            declared,
        } = self
        else {
            return Ok(None);
        };
        declared
            .get_or_try_init(|| async {
                docs.ensure_collection(&CollectionSpec::new(REGISTRY_COLLECTION))
                    .await?;
                // The cluster scope is outside boot's per-tenant migration.
                migrate_cluster(scoped).await
            })
            .await
            .map_err(|e| format!("profile registry: {e}"))?;
        Ok(Some(docs))
    }

    /// The record of profile `id`, or `None` when it is not provisioned.
    ///
    /// # Errors
    ///
    /// When the record cannot be read or parsed.
    pub async fn get(&self, id: &ProfileId) -> Result<Option<ProfileMeta>, String> {
        if let Some(docs) = self.docs().await? {
            let key = key_of(id.as_str()).map_err(|e| format!("profile {id}: {e}"))?;
            let Some(stored) = docs
                .get(REGISTRY_COLLECTION, &key)
                .await
                .map_err(|e| format!("reading profile {id}: {e}"))?
            else {
                return Ok(None);
            };
            return meta_of(&stored)
                .map(Some)
                .map_err(|e| format!("parsing profile {id}: {e}"));
        }
        let Self::Files { root } = self else {
            unreachable!("a registry is documents or files");
        };
        read_meta(&ProfileLayout::new(root, id))
    }

    /// Records `meta` unless its profile is already recorded. Returns whether
    /// it was new.
    ///
    /// # Errors
    ///
    /// When the record cannot be written.
    pub async fn create(&self, meta: &ProfileMeta) -> Result<bool, String> {
        let id = &meta.profile_id;
        if let Some(docs) = self.docs().await? {
            let doc = body_of(meta).map_err(|e| format!("encoding profile {id}: {e}"))?;
            let key = key_of(id.as_str()).map_err(|e| format!("profile {id}: {e}"))?;
            let created_ms = i64::try_from(meta.created_at)
                .unwrap_or(i64::MAX / 1000)
                .saturating_mul(1000);
            return match docs
                .put_with(
                    REGISTRY_COLLECTION,
                    &key,
                    doc,
                    Precondition::Absent,
                    WriteOptions::new().created_ms(created_ms),
                )
                .await
            {
                Ok(_) => Ok(true),
                Err(e) if e.kind() == ErrorKind::Conflict => Ok(false),
                Err(e) => Err(format!("recording profile {id}: {e}")),
            };
        }
        let Self::Files { root } = self else {
            unreachable!("a registry is documents or files");
        };
        let layout = ProfileLayout::new(root, id);
        if layout.meta_path.exists() {
            return Ok(false);
        }
        let raw = toml::to_string(meta).map_err(|e| format!("encoding profile meta: {e}"))?;
        std::fs::write(&layout.meta_path, raw)
            .map_err(|e| format!("writing {}: {e}", layout.meta_path.display()))?;
        Ok(true)
    }

    /// Forgets profile `id`. Returns whether it was recorded. The files
    /// registry forgets a profile when its directory is archived, so this
    /// only reports there.
    ///
    /// # Errors
    ///
    /// When the record cannot be removed.
    pub async fn remove(&self, id: &ProfileId) -> Result<bool, String> {
        if let Some(docs) = self.docs().await? {
            let key = key_of(id.as_str()).map_err(|e| format!("profile {id}: {e}"))?;
            return docs
                .delete(REGISTRY_COLLECTION, &key, Precondition::None)
                .await
                .map_err(|e| format!("forgetting profile {id}: {e}"));
        }
        Ok(self.get(id).await?.is_some())
    }

    /// Every recorded profile, sorted by id. A record that cannot be read is
    /// logged and skipped, so one bad profile never hides the rest.
    ///
    /// # Errors
    ///
    /// When the registry itself cannot be read.
    pub async fn list(&self) -> Result<Vec<ProfileMeta>, String> {
        let mut found = Vec::new();
        if let Some(docs) = self.docs().await? {
            let stored = docs
                .query_all(REGISTRY_COLLECTION, &Query::all())
                .await
                .map_err(|e| format!("listing profiles: {e}"))?;
            for doc in stored {
                match meta_of(&doc) {
                    Ok(meta) => found.push(meta),
                    Err(error) => {
                        log::warn!("[profiles][registry] skipping record {}: {error}", doc.id);
                    }
                }
            }
        } else {
            let Self::Files { root } = self else {
                unreachable!("a registry is documents or files");
            };
            let dir = layout::users_dir(root);
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
                Err(e) => return Err(format!("reading {}: {e}", dir.display())),
            };
            for entry in entries.flatten() {
                let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                let Ok(id) = ProfileId::parse(&name) else {
                    continue;
                };
                match read_meta(&ProfileLayout::new(root, &id)) {
                    Ok(Some(meta)) => found.push(meta),
                    Ok(None) => {}
                    Err(error) => log::warn!("[profiles][registry] skipping profile={id}: {error}"),
                }
            }
        }
        found.sort_by(|a, b| a.profile_id.cmp(&b.profile_id));
        Ok(found)
    }
}

fn read_meta(layout: &ProfileLayout) -> Result<Option<ProfileMeta>, String> {
    let raw = match std::fs::read_to_string(&layout.meta_path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("reading {}: {e}", layout.meta_path.display())),
    };
    toml::from_str(&raw)
        .map(Some)
        .map_err(|e| format!("parsing {}: {e}", layout.meta_path.display()))
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
