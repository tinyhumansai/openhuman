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
//! serde value unwrapped. The one-time import of a legacy
//! `integrations/<name>` file is a `tsd.migrations` entry (owner `composio`,
//! [`crate::storage::import_state`]), so the file itself is never touched.
//!
//! `Kv` has no read-modify-write, so [`Docs::update`] runs tsd's retrying
//! compare-and-swap ([`tinystoragedrivers::Repo::update`]) over the same
//! `tsd.kv` document; [`KvEntry`] is that document's body, and the tests pin
//! it to what `Kv` reads and writes.

use std::path::Path;

use anyhow::Result;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::storage::documents::Repo;
use crate::storage::import_state;
use crate::storage::state_kv::StateKv;

/// The `tsd.kv` owner and namespace of the state values.
const KV_OWNER: &str = "composio";
const KV_NS: &str = "state";
/// The `tsd.migrations` owner of the legacy-file imports.
const IMPORT_OWNER: &str = "composio";
/// The old marker collection, now the `kind` of an import's step.
const IMPORT_KIND: &str = "composio_state_import";
const DOMAIN: &str = "composio::file_store";

/// The document store for this call: the installed backend, else the
/// workspace default, in the acting scope.
pub(super) fn current(workspace_dir: &Path) -> Result<Docs> {
    #[cfg(test)]
    if let Some(docs) = TEST_OVERRIDE.with(|slot| slot.borrow().clone()) {
        return Ok(docs);
    }
    Ok(Docs {
        repo: Repo::current_for(workspace_dir, DOMAIN, Vec::new)?,
        state: StateKv::open(workspace_dir, KV_OWNER, KV_NS)?,
    })
}

#[cfg(test)]
thread_local! {
    static TEST_OVERRIDE: std::cell::RefCell<Option<Docs>> = const { std::cell::RefCell::new(None) };
}

/// The composio state over one scoped document handle.
#[derive(Clone)]
pub(super) struct Docs {
    repo: Repo,
    state: StateKv,
}

impl Docs {
    #[cfg(test)]
    pub(super) fn over(scoped: &crate::storage::Storage) -> Self {
        Self {
            repo: Repo::over(scoped, DOMAIN, Vec::new),
            state: StateKv::over(scoped, KV_OWNER, KV_NS),
        }
    }

    #[cfg(test)]
    pub(super) fn on_backend(
        backend: &std::sync::Arc<dyn crate::storage::StorageBackend>,
        scope: &crate::storage::Scope,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            repo: Repo::on(backend, scope, DOMAIN, Vec::new)?,
            state: StateKv::on_backend(backend, scope, KV_OWNER, KV_NS)?,
        })
    }

    /// The value stored stored under `name`, or `T::default()`.
    pub(super) fn load<T: DeserializeOwned + Default>(&self, name: &str) -> Result<T> {
        let stored: Option<Value> = self.state.get(name)?;
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
        self.state.update(name, change)
    }

    /// Whether the one-time import of the legacy `integrations/<name>` file
    /// finished in this scope.
    pub(super) fn legacy_imported(&self, name: &str) -> Result<bool> {
        import_state::is_done(&self.repo, IMPORT_OWNER, &import_step(name))
    }

    /// Stores `value` under `name` only when no value is stored there yet
    /// (the one-time import of a pre-documents state file), then records the
    /// import in `tsd.migrations` so it never runs again. The legacy file is
    /// not modified. Returns whether the value was written.
    pub(super) fn import_if_absent(&self, name: &str, value: serde_json::Value) -> Result<bool> {
        log::debug!("[composio:store] document import id={name}");
        let step = import_step(name);
        let repo = self.repo.clone();
        let body_repo = repo.clone();
        let body_step = step.clone();
        let claimed = import_state::with_claim(&repo, IMPORT_OWNER, &step, move |condition| {
            let claimed_repo = body_repo.conditioned(condition)?;
            let state = StateKv::over(claimed_repo.scoped(), KV_OWNER, KV_NS);
            let wrote = state.insert_if_absent(name, &value)?;
            import_state::set_done(&claimed_repo, IMPORT_OWNER, &body_step)?;
            Ok(wrote)
        })?;
        Ok(claimed.unwrap_or(false))
    }

    #[cfg(test)]
    pub(super) fn save<T: Serialize>(&self, name: &str, value: &T) -> Result<()> {
        log::debug!("[composio:store] document save id={name}");
        self.state.put(name, value)
    }
}

fn import_step(name: &str) -> String {
    import_state::step_id(IMPORT_KIND, name)
}

#[cfg(test)]
#[path = "file_store_documents_tests.rs"]
mod tests;
