//! The Composio host state files on the `tinystoragedrivers` document port.
//!
//! `file_store.rs` opens it per call, so the state lives under the acting
//! agent's storage scope (`local` on a single-user host) in the installed
//! backend, or the workspace default when none is installed. These are the connected-account identities
//! and the per-toolkit agent scope preferences: user data, so unlike a cache
//! they must follow the user to a shared backend.
//!
//! # Layout
//!
//! Each named value is one entry of tsd's key-value primitive
//! (`tsd.kv`, owner `composio`, namespace `state`, [`tinystoragedrivers::Kv`]),
//! keyed by the value name (`composio_identities.json`, ...), holding the
//! serde value unwrapped. v1 kept it in a `composio_state` collection as
//! `{"value": ...}`; [`migrate_legacy_state`] moves those at boot. The
//! one-time import of a legacy `integrations/<name>` file is a
//! `tsd.migrations` entry (owner `composio`,
//! [`crate::storage::import_state`]), so the file itself is never touched.
//!
//! `Kv` has no read-modify-write, so [`Docs::update`] runs tsd's retrying
//! compare-and-swap ([`tinystoragedrivers::Repo::update`]) over the same
//! `tsd.kv` document; [`KvEntry`] is that document's body, and the tests pin
//! it to what `Kv` reads and writes.

use std::path::Path;

use anyhow::Result;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tinystoragedrivers::{
    CollectionSpec, DocumentStore, DocumentStoreExt, Key, Kv, ScopedStorage, StorageError,
};

use crate::storage::documents::Repo;
use crate::storage::import_state;

/// The `tsd.kv` owner and namespace of the state values.
const KV_OWNER: &str = "composio";
const KV_NS: &str = "state";
/// The `tsd.migrations` owner of the legacy-file imports.
const IMPORT_OWNER: &str = "composio";
/// The old marker collection, now the `kind` of an import's step.
const IMPORT_KIND: &str = "composio_state_import";
/// The v1 collection, `{"value": ...}` per document.
const LEGACY_STATE: &str = "composio_state";
const DOMAIN: &str = "composio::file_store";

fn collections() -> Vec<CollectionSpec> {
    tinystoragedrivers::primitives::collection_specs()
}

/// The body of a `tsd.kv` document (`{owner, ns, key, value}`), as `Kv` writes it.
#[derive(Serialize, Deserialize)]
struct KvEntry {
    owner: String,
    ns: String,
    key: String,
    value: Value,
}

/// The `tsd.kv` document id of value `name`, as `Kv` derives it.
fn kv_id(name: &str) -> Result<String, StorageError> {
    Ok(Key::of([KV_OWNER, KV_NS, name])?.as_str().to_owned())
}

fn kv(docs: &std::sync::Arc<dyn DocumentStore>) -> Result<Kv, StorageError> {
    Kv::ns(docs, KV_OWNER, KV_NS)
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

#[cfg(test)]
thread_local! {
    static TEST_OVERRIDE: std::cell::RefCell<Option<Docs>> = const { std::cell::RefCell::new(None) };
}

/// The composio state over one scoped document handle.
#[derive(Clone)]
pub(super) struct Docs(Repo);

impl Docs {
    #[cfg(test)]
    pub(super) fn over(scoped: &crate::storage::ScopedStorage) -> Self {
        Self(Repo::over(scoped, DOMAIN, collections))
    }

    /// The value stored stored under `name`, or `T::default()`.
    pub(super) fn load<T: DeserializeOwned + Default>(&self, name: &str) -> Result<T> {
        let name = name.to_string();
        let stored: Option<Value> = self
            .0
            .run(|docs| async move { kv(&docs)?.get::<Value>(&name).await })?;
        match stored {
            Some(value) => Ok(serde_json::from_value(value)?),
            None => Ok(T::default()),
        }
    }

    /// Read-modify-write of the value stored under `name`, under
    /// compare-and-swap so two cores sharing the scope never lose each other's
    /// change. `change` returns its result and whether it changed the value
    /// (an unchanged value is not written); it is retried on a conflict, so it
    /// must be repeatable.
    pub(super) fn update<T, R>(
        &self,
        name: &str,
        change: impl Fn(&mut T) -> (R, bool) + Send + Sync + 'static,
    ) -> Result<R>
    where
        T: DeserializeOwned + Serialize + Default + Send + 'static,
        R: Send + 'static,
    {
        let name = name.to_string();
        self.0.run(|docs| async move {
            let repo = tinystoragedrivers::Repo::<KvEntry>::new(
                &docs,
                tinystoragedrivers::primitives::KV,
            )?;
            let mut result = None;
            repo.update(&kv_id(&name)?, |current| {
                let mut value: T = match &current {
                    Some(entry) => serde_json::from_value(entry.value.clone())
                        .map_err(|e| StorageError::invalid_input(e.to_string()))?,
                    None => T::default(),
                };
                let (outcome, changed) = change(&mut value);
                result = Some(outcome);
                if !changed {
                    return Ok(None);
                }
                Ok(Some(KvEntry {
                    owner: KV_OWNER.to_owned(),
                    ns: KV_NS.to_owned(),
                    key: name.clone(),
                    value: serde_json::to_value(&value)
                        .map_err(|e| StorageError::invalid_input(e.to_string()))?,
                }))
            })
            .await?;
            result.ok_or_else(|| StorageError::backend("composio state update produced no result"))
        })
    }

    /// Whether the one-time import of the legacy `integrations/<name>` file
    /// finished in this scope.
    pub(super) fn legacy_imported(&self, name: &str) -> Result<bool> {
        import_state::is_done(&self.0, IMPORT_OWNER, &import_step(name))
    }

    /// Stores `value` under `name` only when no value is stored there yet
    /// (the one-time import of a pre-documents state file), then records the
    /// import in `tsd.migrations` so it never runs again. The legacy file is
    /// not modified. Returns whether the value was written.
    pub(super) fn import_if_absent(&self, name: &str, value: serde_json::Value) -> Result<bool> {
        log::debug!("[composio:store] document import id={name}");
        let step = import_step(name);
        let claimed = import_state::with_claim(&self.0, IMPORT_OWNER, &step, || {
            let (id, entry) = (
                kv_id(name)?,
                KvEntry {
                    owner: KV_OWNER.to_owned(),
                    ns: KV_NS.to_owned(),
                    key: name.to_owned(),
                    value: value.clone(),
                },
            );
            let wrote = self.0.run(|docs| async move {
                let repo = tinystoragedrivers::Repo::<KvEntry>::new(
                    &docs,
                    tinystoragedrivers::primitives::KV,
                )?;
                match repo.insert(&id, &entry).await {
                    Ok(_) => Ok(true),
                    Err(error) if error.kind() == tinystoragedrivers::ErrorKind::Conflict => {
                        Ok(false)
                    }
                    Err(error) => Err(error),
                }
            })?;
            import_state::set_done(&self.0, IMPORT_OWNER, &step)?;
            Ok(wrote)
        })?;
        Ok(claimed.unwrap_or(false))
    }

    #[cfg(test)]
    pub(super) fn save<T: Serialize>(&self, name: &str, value: &T) -> Result<()> {
        let name = name.to_string();
        let value = serde_json::to_value(value)?;
        log::debug!("[composio:store] document save id={name}");
        self.0
            .run(|docs| async move { kv(&docs)?.put(&name, &value).await.map(|_| ()) })
    }
}

fn import_step(name: &str) -> String {
    import_state::step_id(IMPORT_KIND, name)
}

/// Moves the v1 `composio_state` documents (`{"value": ...}`) into `tsd.kv`
/// (owner `composio`, namespace `state`), then drops the collection. A value
/// already in `tsd.kv` wins: it is newer. Returns the values moved.
///
/// A document step cannot write another owner's collection, so this is the
/// boot-time importer that stands in for the step the catalogue lists for
/// `composio_state` (schema-v2.md, host stores).
///
/// # Errors
///
/// A storage error; the values not yet moved stay in place for the next boot.
pub(crate) async fn migrate_legacy_state(scoped: &ScopedStorage) -> Result<usize, StorageError> {
    use tinystoragedrivers::Capability;
    let docs = scoped.documents();
    if !docs
        .capabilities()
        .contains(Capability::CollectionEnumeration)
        || !docs
            .collections("")
            .await?
            .iter()
            .any(|name| name == LEGACY_STATE)
    {
        return Ok(0);
    }
    for spec in collections() {
        docs.ensure_collection(&spec).await?;
    }
    let store = kv(docs)?;
    let mut moved = 0;
    for legacy in docs
        .query_all(LEGACY_STATE, &tinystoragedrivers::Query::all())
        .await?
    {
        let Some(value) = legacy.doc.get("value") else {
            continue;
        };
        if store.get::<Value>(&legacy.id).await?.is_none() {
            store.put(&legacy.id, value).await?;
            moved += 1;
        }
    }
    docs.drop_collection(LEGACY_STATE).await?;
    log::info!("[composio:store] moved {moved} legacy state values to tsd.kv");
    Ok(moved)
}

#[cfg(test)]
#[path = "file_store_documents_tests.rs"]
mod tests;
