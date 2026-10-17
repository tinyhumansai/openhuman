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
//! | Collection | Document id | Holds |
//! | --- | --- | --- |
//! | `composio_state` | the value name (`composio_identities.json`, ...) | `value`: the serde value |
//! | `composio_state_import` | the value name | marker: the legacy `integrations/<name>` file was imported (the file is left byte-identical) |

use std::path::Path;

use anyhow::Result;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::json;
use tinystoragedrivers::{CollectionSpec, ErrorKind, Precondition, StorageError};

use crate::storage::documents::Repo;

const STATE: &str = "composio_state";
/// One marker document per legacy file whose one-time import finished, so the
/// file itself is never touched.
const IMPORTED: &str = "composio_state_import";
const DOMAIN: &str = "composio::file_store";

fn collections() -> Vec<CollectionSpec> {
    vec![CollectionSpec::new(STATE), CollectionSpec::new(IMPORTED)]
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
        let id = name.to_string();
        let stored = self
            .0
            .run(|docs| async move { docs.get(STATE, &id).await })?;
        match stored.and_then(|stored| stored.doc.get("value").cloned()) {
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
        change: impl Fn(&mut T) -> (R, bool) + Send + 'static,
    ) -> Result<R>
    where
        T: DeserializeOwned + Serialize + Default + Send + 'static,
        R: Send + 'static,
    {
        let id = name.to_string();
        self.0.run(|docs| async move {
            for _ in 0..crate::storage::documents::CAS_ATTEMPTS {
                let stored = docs.get(STATE, &id).await?;
                let (mut value, precondition) = match &stored {
                    Some(stored) => (
                        stored
                            .doc
                            .get("value")
                            .cloned()
                            .map(serde_json::from_value)
                            .transpose()
                            .map_err(|e| StorageError::invalid_input(e.to_string()))?
                            .unwrap_or_default(),
                        stored.unchanged(),
                    ),
                    None => (T::default(), Precondition::Absent),
                };
                let (result, changed) = change(&mut value);
                if !changed {
                    return Ok(result);
                }
                let doc = json!({
                    "value": serde_json::to_value(&value)
                        .map_err(|e| StorageError::invalid_input(e.to_string()))?
                });
                match docs.put(STATE, &id, doc, precondition).await {
                    Ok(_) => return Ok(result),
                    Err(error) if error.kind() == ErrorKind::Conflict => {}
                    Err(error) => return Err(error),
                }
            }
            Err(StorageError::conflict(format!(
                "{STATE}/{id} kept changing under {} attempts",
                crate::storage::documents::CAS_ATTEMPTS
            )))
        })
    }

    /// Whether the one-time import of the legacy `integrations/<name>` file
    /// finished in this scope.
    pub(super) fn legacy_imported(&self, name: &str) -> Result<bool> {
        let id = name.to_string();
        let found = self
            .0
            .run(|docs| async move { docs.get(IMPORTED, &id).await })?;
        Ok(found.is_some())
    }

    /// Stores `value` under `name` only when no value is stored there yet
    /// (the one-time import of a pre-documents state file), then records the
    /// import in a marker document so it never runs again. The legacy file is
    /// not modified. Returns whether the value was written.
    pub(super) fn import_if_absent(&self, name: &str, value: serde_json::Value) -> Result<bool> {
        let id = name.to_string();
        let doc = json!({ "value": value });
        log::debug!("[composio:store] document import id={id}");
        let claimed =
            crate::storage::import_claim::with_claim(&self.0, IMPORTED, &id.clone(), || {
                let (id, doc) = (id.clone(), doc.clone());
                self.0.run(|docs| async move {
                    let wrote = match docs.put(STATE, &id, doc, Precondition::Absent).await {
                        Ok(_) => true,
                        Err(error) if error.kind() == ErrorKind::Conflict => false,
                        Err(error) => return Err(error),
                    };
                    match docs
                        .put(
                            IMPORTED,
                            &id,
                            json!({ "imported": true }),
                            Precondition::Absent,
                        )
                        .await
                    {
                        Ok(_) => {}
                        // Another core finished first.
                        Err(error) if error.kind() == ErrorKind::Conflict => {}
                        Err(error) => return Err(error),
                    }
                    Ok(wrote)
                })
            })?;
        Ok(claimed.unwrap_or(false))
    }

    #[cfg(test)]
    pub(super) fn save<T: Serialize>(&self, name: &str, value: &T) -> Result<()> {
        let id = name.to_string();
        let doc = json!({ "value": serde_json::to_value(value)? });
        log::debug!("[composio:store] document save id={id}");
        self.0.run(|docs| async move {
            docs.put(STATE, &id, doc, Precondition::None)
                .await
                .map(|_| ())
        })
    }
}

#[cfg(test)]
#[path = "file_store_documents_tests.rs"]
mod tests;
