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
use sha2::Digest;
use tinystoragedrivers::{CollectionSpec, Cursor, ErrorKind, Precondition, StorageError};

use super::documents::Repo;
use super::import_state;
use super::{Scope, Storage};

/// The `tsd.migrations` step kind of a state-file import.
const IMPORT_KIND: &str = "state_file";

/// The envelope used to preserve exact host namespace/key identity in TSD KV.
#[derive(Serialize, Deserialize)]
struct KvEntry {
    owner: String,
    ns: String,
    key: String,
    value: Value,
}

fn collections() -> Vec<CollectionSpec> {
    Vec::new()
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
        // State-file adoption uses TSD's cross-process conditional writes.
        // The ordinary unit-test fallback is the in-memory backend, which
        // deliberately does not claim that capability; use the same durable
        // SQLite default as production for this path when available.
        #[cfg(all(test, feature = "storage-sqlite"))]
        super::local::use_persistent_default_for_test(workspace_dir);

        let repo = Repo::current_for(workspace_dir, "storage::state_kv", collections)?;
        Ok(Self { repo, owner, ns })
    }

    /// The state for an explicit backend and scope, for importers that must
    /// keep data on the backend they are migrating rather than reopen a URL.
    pub(crate) fn on_backend(
        backend: &Arc<dyn super::StorageBackend>,
        scope: &Scope,
        owner: &'static str,
        ns: &'static str,
    ) -> Result<Self> {
        let repo = Repo::on(backend, scope, "storage::state_kv", collections)?;
        Ok(Self { repo, owner, ns })
    }

    /// The state over an already scoped handle (tests, explicit scopes).
    #[must_use]
    pub fn over(scoped: &Storage, owner: &'static str, ns: &'static str) -> Self {
        Self {
            repo: Repo::over(scoped, "storage::state_kv", collections),
            owner,
            ns,
        }
    }

    fn prefix(&self) -> String {
        let namespace = format!("{}\0{}", self.owner, self.ns);
        format!(
            "openhuman-state:{}:",
            hex::encode(sha2::Sha256::digest(namespace.as_bytes()))
        )
    }

    fn id(&self, name: &str) -> Result<String, StorageError> {
        Ok(format!(
            "{}{}",
            self.prefix(),
            hex::encode(sha2::Sha256::digest(name.as_bytes()))
        ))
    }

    /// The value stored under `name`.
    ///
    /// # Errors
    ///
    /// A storage error, or a stored value that is not a `T`.
    pub fn get<T: DeserializeOwned + Send + 'static>(&self, name: &str) -> Result<Option<T>> {
        let (this, name) = (self.clone(), name.to_owned());
        self.repo.run_storage(move |storage| async move {
            let Some(record) = storage.kv().get(&this.id(&name)?).await? else {
                return Ok(None);
            };
            let entry = this.decode_entry(&record.id, record.doc)?;
            serde_json::from_value(entry.value)
                .map(Some)
                .map_err(|error| StorageError::invalid_input(error.to_string()))
        })
    }

    /// Stores `value` under `name`, replacing any earlier one.
    ///
    /// # Errors
    ///
    /// A storage error.
    pub fn put<T: Serialize>(&self, name: &str, value: &T) -> Result<()> {
        let (this, name) = (self.clone(), name.to_owned());
        let value = serde_json::to_value(value).context("serialize the state value")?;
        self.repo.run_storage(move |storage| async move {
            storage
                .kv()
                .put(&this.id(&name)?, this.entry(&name, value)?)
                .await?;
            Ok(())
        })
    }

    /// Removes `name`, reporting whether it existed.
    ///
    /// # Errors
    ///
    /// A storage error.
    pub fn delete(&self, name: &str) -> Result<bool> {
        let (this, name) = (self.clone(), name.to_owned());
        self.repo.run_storage(move |storage| async move {
            match storage.kv().get(&this.id(&name)?).await? {
                Some(current) => storage.kv().delete(&current.id, current.unchanged()).await,
                None => Ok(false),
            }
        })
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
        self.repo.run_storage(move |storage| async move {
            let mut cursor: Option<Cursor> = None;
            let mut entries = Vec::new();
            loop {
                let page = storage.kv().list(&this.prefix(), cursor, 256).await?;
                for record in page.items {
                    let entry = this.decode_entry(&record.id, record.doc)?;
                    if entry.key.starts_with(&prefix) {
                        let value = serde_json::from_value(entry.value)
                            .map_err(|error| StorageError::invalid_input(error.to_string()))?;
                        entries.push((entry.key, value));
                    }
                }
                cursor = page.next;
                if cursor.is_none() {
                    break;
                }
            }
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            Ok(entries)
        })
    }

    /// Stores `value` under `name` only when nothing is stored there yet.
    /// Returns whether it was written.
    ///
    /// # Errors
    ///
    /// A storage error.
    pub fn insert_if_absent<T: Serialize>(&self, name: &str, value: &T) -> Result<bool> {
        let body = self.entry(
            name,
            serde_json::to_value(value).context("serialize the state value")?,
        )?;
        let this = self.clone();
        let name = name.to_owned();
        self.repo.run_storage(move |storage| async move {
            match storage
                .kv()
                .cas(&this.id(&name)?, body, Precondition::Absent)
                .await
            {
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
        self.repo.run_storage(move |storage| async move {
            for _ in 0..32 {
                let current = storage.kv().get(&this.id(&name)?).await?;
                let mut value: T = match &current {
                    Some(record) => {
                        let entry = this.decode_entry(&record.id, record.doc.clone())?;
                        serde_json::from_value(entry.value)
                            .map_err(|error| StorageError::invalid_input(error.to_string()))?
                    }
                    None => T::default(),
                };
                let (outcome, changed) = change(&mut value);
                if !changed {
                    return Ok(outcome);
                }
                let body = this.entry(
                    &name,
                    serde_json::to_value(&value)
                        .map_err(|error| StorageError::serialization(error.to_string()))?,
                )?;
                let expected = current
                    .as_ref()
                    .map_or(Precondition::Absent, |record| record.unchanged());
                match storage.kv().cas(&this.id(&name)?, body, expected).await {
                    Ok(_) => return Ok(outcome),
                    Err(error) if error.kind() == ErrorKind::Conflict => continue,
                    Err(error) => return Err(error),
                }
            }
            Err(StorageError::conflict(
                "state update exceeded its retry budget",
            ))
        })
    }

    /// Async read-modify-write variant for callbacks already running on the
    /// storage executor. It avoids re-entering the synchronous storage bridge.
    pub async fn update_async<T, R>(
        &self,
        name: &str,
        change: impl Fn(&mut T) -> (R, bool) + Send + Sync,
    ) -> Result<R>
    where
        T: DeserializeOwned + Serialize + Default,
    {
        let storage = self.repo.scoped().clone();
        for _ in 0..32 {
            let current = storage.kv().get(&self.id(name)?).await?;
            let mut value: T = match &current {
                Some(record) => {
                    let entry = self.decode_entry(&record.id, record.doc.clone())?;
                    serde_json::from_value(entry.value)
                        .map_err(|error| StorageError::invalid_input(error.to_string()))?
                }
                None => T::default(),
            };
            let (outcome, changed) = change(&mut value);
            if !changed {
                return Ok(outcome);
            }
            let body = self.entry(
                name,
                serde_json::to_value(&value)
                    .map_err(|error| StorageError::serialization(error.to_string()))?,
            )?;
            let expected = current
                .as_ref()
                .map_or(Precondition::Absent, |record| record.unchanged());
            match storage.kv().cas(&self.id(name)?, body, expected).await {
                Ok(_) => return Ok(outcome),
                Err(error) if error.kind() == ErrorKind::Conflict => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(StorageError::conflict("state update exceeded its retry budget").into())
    }

    fn entry(&self, key: &str, value: Value) -> Result<Value, StorageError> {
        serde_json::to_value(KvEntry {
            owner: self.owner.to_owned(),
            ns: self.ns.to_owned(),
            key: key.to_owned(),
            value,
        })
        .map_err(|error| StorageError::serialization(error.to_string()))
    }

    fn decode_entry(&self, storage_key: &str, value: Value) -> Result<KvEntry, StorageError> {
        let entry: KvEntry = serde_json::from_value(value)
            .map_err(|error| StorageError::invalid_input(error.to_string()))?;
        if entry.owner != self.owner || entry.ns != self.ns || self.id(&entry.key)? != storage_key {
            return Err(StorageError::invalid_input(
                "state KV identity envelope does not match its storage prefix",
            ));
        }
        Ok(entry)
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

    /// Imports a typed legacy JSON value once, preserving invalid input beside
    /// the source as `<name>.json.corrupt` so the state port can start fresh.
    /// Existing generic and strict import paths keep their original behavior.
    ///
    /// # Errors
    ///
    /// A storage error, a legacy read error, or a failure to quarantine an
    /// invalid legacy file.
    pub fn import_file_quarantining<T: DeserializeOwned>(
        &self,
        name: &str,
        legacy: &Path,
    ) -> Result<()> {
        self.import_once(name, |state| {
            match std::fs::read(legacy) {
                Ok(bytes) if bytes.iter().all(u8::is_ascii_whitespace) => {}
                Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                    Ok(value) if serde_json::from_value::<T>(value.clone()).is_ok() => {
                        let wrote = state.insert_if_absent(name, &value)?;
                        log::debug!(
                            "[storage::state_kv] imported {}/{}/{name} (written={wrote})",
                            self.owner,
                            self.ns
                        );
                    }
                    Ok(_) => log::warn!(
                        "[storage::state_kv] legacy {}/{}/{name} has the wrong value type; quarantining",
                        self.owner,
                        self.ns
                    ),
                    Err(error) => log::warn!(
                        "[storage::state_kv] legacy {}/{}/{name} is not JSON ({error}); quarantining",
                        self.owner,
                        self.ns
                    ),
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("read the legacy state file {}", legacy.display()));
                }
            }
            Ok(())
        })?;
        // Validating the legacy file reparses it, so do it once per backend
        // instance, scope, state name and requested value type; a failed
        // validation is retried by the next call.
        let key = format!(
            "quarantine/{}/{}/{name}/{}",
            self.owner,
            self.ns,
            std::any::type_name::<T>()
        );
        self.repo
            .once_per_backend(&key, || self.quarantine_invalid_legacy::<T>(legacy))
    }

    fn quarantine_invalid_legacy<T: DeserializeOwned>(&self, legacy: &Path) -> Result<()> {
        let bytes = match std::fs::read(legacy) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("read the legacy state file {}", legacy.display()));
            }
        };
        if bytes.iter().all(u8::is_ascii_whitespace)
            || serde_json::from_slice::<Value>(&bytes)
                .and_then(|value| serde_json::from_value::<T>(value))
                .is_ok()
        {
            return Ok(());
        }

        let backup = legacy.with_extension("json.corrupt");
        if backup.exists() {
            anyhow::bail!(
                "the corrupt legacy backup {} already exists",
                backup.display()
            );
        }
        std::fs::rename(legacy, &backup)
            .with_context(|| format!("quarantine corrupt legacy state {}", legacy.display()))
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
    pub fn import_once(&self, name: &str, body: impl FnOnce(&StateKv) -> Result<()>) -> Result<()> {
        let key = format!("{}/{}/{name}", self.owner, self.ns);
        let step = import_state::step_id(IMPORT_KIND, &key);
        self.repo.once_per_backend(&key, || {
            if import_state::is_done(&self.repo, self.owner, &step)? {
                return Ok(());
            }
            import_state::with_claim(&self.repo, self.owner, &step, |condition| {
                let claimed_repo = self.repo.conditioned(condition)?;
                let claimed_state = Self {
                    repo: claimed_repo.clone(),
                    owner: self.owner,
                    ns: self.ns,
                };
                if import_state::is_done(&claimed_repo, self.owner, &step)? {
                    return Ok(());
                }
                body(&claimed_state)?;
                import_state::set_done(&claimed_repo, self.owner, &step)
            })?;
            Ok(())
        })
    }

    fn import_file_with(&self, name: &str, legacy: &Path, strict: bool) -> Result<()> {
        self.import_once(name, |state| {
            match std::fs::read(legacy) {
                Ok(bytes) if bytes.iter().all(u8::is_ascii_whitespace) => {}
                Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                    Ok(value) => {
                        let wrote = state.insert_if_absent(name, &value)?;
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
