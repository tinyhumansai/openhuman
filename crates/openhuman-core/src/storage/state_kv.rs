//! Runtime state values on tsd's key-value primitive, with a one-shot import of
//! the legacy JSON file each one lived in.
//!
//! Several domains kept a small piece of runtime state as one JSON file under
//! the workspace (`memory/jobs.json`, the sub-agent lifecycle records, the
//! computer site memory, ...). Behind the storage port that state follows the
//! backend setting and the acting scope like every other record, so a shared
//! backend shares it and a tenant cannot see another's.
//!
//! Each value is one `tsd.kv` entry (`owner`, `ns`, `name`) holding the serde
//! value unwrapped, in the acting scope ([`Repo::current_for`]). A domain
//! passes the legacy file next to the read ([`StateKv::import_file`]): the
//! first read in a backend copies it in with `Precondition::Absent` semantics
//! (an existing entry is newer and wins), records a `tsd.migrations` entry
//! ([`import_state`]) so a value deleted afterwards is not resurrected, and
//! leaves the file untouched (archiving it is the legacy-archive step's job).
//!
//! `Kv` has no read-modify-write, so [`StateKv::update`] runs tsd's retrying
//! compare-and-swap over the same `tsd.kv` document; [`KvEntry`] is that
//! document's body and the tests pin it to what `Kv` reads and writes.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tinystoragedrivers::{
    CollectionSpec, DocumentStore, ErrorKind, Key, Kv, ScopedStorage, StorageError,
};

use super::documents::Repo;
use super::import_state;

/// The `tsd.migrations` step kind of a state-file import.
const IMPORT_KIND: &str = "state_file";

/// The body of a `tsd.kv` document, as `Kv` writes it.
#[derive(Serialize, Deserialize)]
struct KvEntry {
    owner: String,
    ns: String,
    key: String,
    value: Value,
}

fn collections() -> Vec<CollectionSpec> {
    tinystoragedrivers::primitives::collection_specs()
}

/// One `(owner, namespace)` of runtime state values in one scope.
#[derive(Clone)]
pub struct StateKv {
    repo: Repo,
    owner: &'static str,
    ns: &'static str,
}

impl StateKv {
    /// The state of `workspace_dir` for this call: the installed backend, else
    /// the workspace default, in the acting scope.
    ///
    /// # Errors
    ///
    /// When the backend cannot be opened or the scope cannot be resolved.
    pub fn open(workspace_dir: &Path, owner: &'static str, ns: &'static str) -> Result<Self> {
        let repo = Repo::current_for(workspace_dir, "storage::state_kv", collections)?;
        Ok(Self { repo, owner, ns })
    }

    /// The state over an already scoped handle (tests, explicit scopes).
    #[must_use]
    pub fn over(scoped: &ScopedStorage, owner: &'static str, ns: &'static str) -> Self {
        Self {
            repo: Repo::over(scoped, "storage::state_kv", collections),
            owner,
            ns,
        }
    }

    fn kv(&self, docs: &Arc<dyn DocumentStore>) -> Result<Kv, StorageError> {
        Kv::ns(docs, self.owner, self.ns)
    }

    fn id(&self, name: &str) -> Result<String, StorageError> {
        Ok(Key::of([self.owner, self.ns, name])?.as_str().to_owned())
    }

    /// The value stored under `name`.
    ///
    /// # Errors
    ///
    /// A storage error, or a stored value that is not a `T`.
    pub fn get<T: DeserializeOwned + Send + 'static>(&self, name: &str) -> Result<Option<T>> {
        let (this, name) = (self.clone(), name.to_owned());
        self.repo
            .run(move |docs| async move { this.kv(&docs)?.get::<T>(&name).await })
    }

    /// Stores `value` under `name`, replacing any earlier one.
    ///
    /// # Errors
    ///
    /// A storage error.
    pub fn put<T: Serialize>(&self, name: &str, value: &T) -> Result<()> {
        let (this, name) = (self.clone(), name.to_owned());
        let value = serde_json::to_value(value).context("serialize the state value")?;
        self.repo
            .run(move |docs| async move { this.kv(&docs)?.put(&name, &value).await.map(|_| ()) })
    }

    /// Removes `name`, reporting whether it existed.
    ///
    /// # Errors
    ///
    /// A storage error.
    pub fn delete(&self, name: &str) -> Result<bool> {
        let (this, name) = (self.clone(), name.to_owned());
        self.repo
            .run(move |docs| async move { this.kv(&docs)?.delete(&name).await })
    }

    /// Every `(name, value)` whose name starts with `prefix`, in name order.
    ///
    /// # Errors
    ///
    /// A storage error, or a stored value that is not a `T`.
    pub fn list<T: DeserializeOwned + Send + 'static>(
        &self,
        prefix: &str,
    ) -> Result<Vec<(String, T)>> {
        let (this, prefix) = (self.clone(), prefix.to_owned());
        self.repo
            .run(move |docs| async move { this.kv(&docs)?.list::<T>(&prefix).await })
    }

    /// Stores `value` under `name` only when nothing is stored there yet.
    /// Returns whether it was written.
    ///
    /// # Errors
    ///
    /// A storage error.
    pub fn insert_if_absent<T: Serialize>(&self, name: &str, value: &T) -> Result<bool> {
        let entry = KvEntry {
            owner: self.owner.to_owned(),
            ns: self.ns.to_owned(),
            key: name.to_owned(),
            value: serde_json::to_value(value).context("serialize the state value")?,
        };
        let this = self.clone();
        let name = name.to_owned();
        self.repo.run(move |docs| async move {
            let repo = tinystoragedrivers::Repo::<KvEntry>::new(
                &docs,
                tinystoragedrivers::primitives::KV,
            )?;
            match repo.insert(&this.id(&name)?, &entry).await {
                Ok(_) => Ok(true),
                Err(error) if error.kind() == ErrorKind::Conflict => Ok(false),
                Err(error) => Err(error),
            }
        })
    }

    /// Read-modify-write of the value under `name` under compare-and-swap, so
    /// two cores sharing the scope never lose each other's change. `change`
    /// returns its result and whether it changed the value (an unchanged value
    /// is not written); it is retried on a conflict, so it must be repeatable.
    ///
    /// # Errors
    ///
    /// A storage error, or a conflict that outlasted the retry budget.
    pub fn update<T, R>(
        &self,
        name: &str,
        change: impl Fn(&mut T) -> (R, bool) + Send + Sync + 'static,
    ) -> Result<R>
    where
        T: DeserializeOwned + Serialize + Default + Send + 'static,
        R: Send + 'static,
    {
        let this = self.clone();
        let name = name.to_owned();
        self.repo.run(move |docs| async move {
            let repo = tinystoragedrivers::Repo::<KvEntry>::new(
                &docs,
                tinystoragedrivers::primitives::KV,
            )?
            .with_retry(tinystoragedrivers::RetryPolicy::default());
            let mut result = None;
            repo.update(&this.id(&name)?, |current| {
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
                    owner: this.owner.to_owned(),
                    ns: this.ns.to_owned(),
                    key: name.clone(),
                    value: serde_json::to_value(&value)
                        .map_err(|e| StorageError::invalid_input(e.to_string()))?,
                }))
            })
            .await?;
            result.ok_or_else(|| StorageError::backend("state update produced no result"))
        })
    }

    /// Copies the legacy JSON file `legacy` into `name` once for this backend
    /// and scope: when no `tsd.migrations` entry says it was imported, the
    /// process holding the claim stores the file's value unless an entry
    /// already exists, then records the import. A missing file imports
    /// nothing but is recorded too, so a file that appears later is not
    /// resurrected over newer state. An unparsable file is logged and
    /// recorded, never retried. The file itself is not touched.
    ///
    /// # Errors
    ///
    /// A storage error.
    pub fn import_file(&self, name: &str, legacy: &Path) -> Result<()> {
        self.import_file_with(name, legacy, false)
    }

    /// [`Self::import_file`] for a value that must not start over from a
    /// damaged file: an unparsable file is an error and is not recorded, so
    /// every read fails closed until the file is repaired or removed.
    ///
    /// # Errors
    ///
    /// A storage error, or an unparsable legacy file.
    pub fn import_file_strict(&self, name: &str, legacy: &Path) -> Result<()> {
        self.import_file_with(name, legacy, true)
    }

    /// Runs `body` once for this backend and scope under the import claim, and
    /// records `name` as imported in `tsd.migrations` afterwards. `body` is
    /// skipped when the entry already exists, so it must write with
    /// insert-if-absent semantics; the entry is not recorded when it fails.
    ///
    /// # Errors
    ///
    /// A storage error, or whatever `body` returns.
    pub fn import_once(&self, name: &str, body: impl FnOnce() -> Result<()>) -> Result<()> {
        let key = format!("{}/{}/{name}", self.owner, self.ns);
        let step = import_state::step_id(IMPORT_KIND, &key);
        self.repo.once_per_backend(&key, || {
            if import_state::is_done(&self.repo, self.owner, &step)? {
                return Ok(());
            }
            import_state::with_claim(&self.repo, self.owner, &step, || {
                if import_state::is_done(&self.repo, self.owner, &step)? {
                    return Ok(());
                }
                body()?;
                import_state::set_done(&self.repo, self.owner, &step)
            })?;
            Ok(())
        })
    }

    fn import_file_with(&self, name: &str, legacy: &Path, strict: bool) -> Result<()> {
        self.import_once(name, || {
            match std::fs::read(legacy) {
                Ok(bytes) if bytes.iter().all(u8::is_ascii_whitespace) => {}
                Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                    Ok(value) => {
                        let wrote = self.insert_if_absent(name, &value)?;
                        log::debug!(
                            "[storage::state_kv] imported {}/{}/{name} (written={wrote})",
                            self.owner,
                            self.ns
                        );
                    }
                    Err(error) if strict => {
                        return Err(error).with_context(|| {
                            format!("the legacy state file {} is not JSON", legacy.display())
                        });
                    }
                    Err(error) => log::warn!(
                        "[storage::state_kv] legacy {}/{}/{name} is not JSON ({error}); skipped",
                        self.owner,
                        self.ns
                    ),
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("read the legacy state file {}", legacy.display())
                    });
                }
            }
            Ok(())
        })
    }
}

#[cfg(test)]
#[path = "state_kv_tests.rs"]
mod tests;
