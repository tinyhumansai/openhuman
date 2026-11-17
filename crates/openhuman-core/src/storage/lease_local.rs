//! [`LocalLeases`]: leases as OS file locks, for hosts without a backend.
//!
//! The holder keeps an exclusive `flock` (`fs2`) on `<root>/<sha256(key) hex>/.lease`
//! open for as long as it holds the key; the OS drops it when the process
//! dies, so liveness needs no clock and these leases never expire
//! (`expires_at_ms == u64::MAX`). The record itself sits beside the lock in
//! `.lease.json`, written only by the lock holder (fsynced temp file +
//! durable rename), so others can name the owner.
//!
//! A free lock over a record that is not released means the previous holder
//! died holding it: the next grant is `previous_unclean`.
//!
//! Locks are per open file, so two instances in one process exclude each
//! other just like two processes do. Only processes on one machine (or one
//! filesystem with working `flock`) are covered; a cluster uses
//! [`super::DocumentLeases`].

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;
use fs2::FileExt;
use tinystoragedrivers::Version;

use super::lease::{local_dir, validate_key, LeaseError, LeaseGrant, LeaseRecord, LeaseStore};
use super::StorageError;

const LOCK_FILE: &str = ".lease";
const RECORD_FILE: &str = ".lease.json";
/// Inspections (`holder`, the doctor's `holder_readonly`) test liveness by
/// taking the lock for an instant, so `acquire` retries a contended lock
/// for up to `PROBE_ATTEMPTS × PROBE_WAIT` before calling the key held.
const PROBE_ATTEMPTS: u32 = 10;
const PROBE_WAIT: std::time::Duration = std::time::Duration::from_millis(5);

struct Holding {
    /// Kept open: closing it drops the lock.
    file: File,
    record: LeaseRecord,
    version: Version,
}

/// Leases kept as file locks under one root. See the module docs.
pub struct LocalLeases {
    root: PathBuf,
    node: String,
    endpoint: Option<String>,
    held: Mutex<HashMap<String, Holding>>,
}

impl std::fmt::Debug for LocalLeases {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalLeases")
            .field("root", &self.root)
            .field("node", &self.node)
            .finish_non_exhaustive()
    }
}

fn io_error(what: &str, path: &Path, error: &io::Error) -> StorageError {
    StorageError::backend(format!("lease {what} {}: {error}", path.display()))
}

fn is_contended(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::WouldBlock
        || error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
}

/// `try_lock_exclusive`, riding out a lock held only by an inspection probe.
/// A blocking sleep: the caller holds the `held` std mutex, so it can't await.
// ponytail: a bounded retry absorbs probes; a probe that takes no lock
// (F_GETLK) would remove the race outright.
fn lock_past_probes(file: &File, key: &str, node: &str) -> io::Result<()> {
    let mut retries = 0;
    let result = loop {
        match file.try_lock_exclusive() {
            Err(error) if is_contended(&error) && retries + 1 < PROBE_ATTEMPTS => {
                #[cfg(test)]
                CONTENDED_KEYS
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(key.to_string());
                retries += 1;
                std::thread::sleep(PROBE_WAIT);
            }
            other => break other,
        }
    };
    if retries > 0 {
        tracing::debug!(
            target: "openhuman::storage::lease",
            key,
            node,
            retries,
            locked = result.is_ok(),
            "[lease] local acquire retried a contended lock"
        );
    }
    result
}

/// Keys whose contended lock `lock_past_probes` retried, so a test can
/// release its stand-in probe lock only once its own `acquire` has met it.
#[cfg(test)]
static CONTENDED_KEYS: Mutex<Vec<String>> = Mutex::new(Vec::new());

impl LocalLeases {
    /// Leases under `root` for `node`.
    pub fn new(root: impl Into<PathBuf>, node: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            node: node.into(),
            endpoint: None,
            held: Mutex::new(HashMap::new()),
        }
    }

    /// Names `endpoint` in the records this instance writes, so a refused
    /// caller can be pointed at the holder.
    #[must_use]
    pub fn with_endpoint(mut self, endpoint: Option<String>) -> Self {
        self.endpoint = endpoint;
        self
    }

    fn held(&self) -> std::sync::MutexGuard<'_, HashMap<String, Holding>> {
        self.held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn open_lock(&self, key: &str) -> Result<File, StorageError> {
        let dir = local_dir(&self.root, key);
        fs::create_dir_all(&dir).map_err(|error| io_error("create", &dir, &error))?;
        let path = dir.join(LOCK_FILE);
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| io_error("open", &path, &error))
    }

    /// The record on disk: `None` only when there is none. An unreadable or
    /// malformed record is an error, never a fresh start, so a fencing epoch
    /// is not reused and crash recovery is not skipped.
    fn read_record(&self, key: &str) -> Result<Option<LeaseRecord>, StorageError> {
        let path = local_dir(&self.root, key).join(RECORD_FILE);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error("read", &path, &error)),
        };
        serde_json::from_slice(&bytes).map(Some).map_err(|error| {
            StorageError::serialization(format!("lease record {}: {error}", path.display()))
        })
    }

    fn write_record(&self, key: &str, record: &LeaseRecord) -> Result<(), StorageError> {
        let path = local_dir(&self.root, key).join(RECORD_FILE);
        let bytes = serde_json::to_vec(record)
            .map_err(|error| StorageError::serialization(error.to_string()))?;
        // fsync + durable rename: a crash must not leave an empty record,
        // which would read as malformed and block the key.
        crate::security::secret_storage::file_store::write_atomic(&path, &bytes).map_err(|error| {
            StorageError::backend(format!("lease write {}: {error}", path.display()))
        })
    }

    /// Checks that `grant` is the one this instance holds and applies
    /// `change` to it.
    fn with_current<T>(
        &self,
        grant: &LeaseGrant,
        change: impl FnOnce(&mut HashMap<String, Holding>) -> Result<T, LeaseError>,
    ) -> Result<T, LeaseError> {
        validate_key(&grant.key)?;
        let mut held = self.held();
        let current = held
            .get(&grant.key)
            .is_some_and(|h| h.record.epoch == grant.epoch && h.version == grant.version);
        if !current {
            tracing::info!(
                target: "openhuman::storage::lease",
                key = %grant.key,
                node = %self.node,
                epoch = grant.epoch,
                "[lease] local grant lost: not the one this instance holds"
            );
            return Err(LeaseError::Lost);
        }
        change(&mut held)
    }
}

#[async_trait]
impl LeaseStore for LocalLeases {
    async fn acquire(&self, key: &str, _now_ms: u64) -> Result<LeaseGrant, LeaseError> {
        validate_key(key)?;
        let mut held = self.held();
        if let Some(holding) = held.get(key) {
            return Ok(LeaseGrant {
                key: key.to_string(),
                epoch: holding.record.epoch,
                version: holding.version,
                expires_at_ms: holding.record.expires_at_ms,
                previous_unclean: false,
            });
        }
        let file = self.open_lock(key)?;
        if let Err(error) = lock_past_probes(&file, key, &self.node) {
            if !is_contended(&error) {
                let path = local_dir(&self.root, key).join(LOCK_FILE);
                return Err(io_error("lock", &path, &error).into());
            }
            // Locked elsewhere: only a missing record reads as an unknown
            // owner; an unreadable or malformed one is a storage error.
            let record = self.read_record(key)?.unwrap_or_else(|| LeaseRecord {
                owner: "unknown".to_string(),
                endpoint: None,
                epoch: 0,
                expires_at_ms: u64::MAX,
                released: false,
            });
            tracing::debug!(
                target: "openhuman::storage::lease",
                key,
                node = %self.node,
                owner = %record.owner,
                "[lease] local acquire refused: locked elsewhere"
            );
            return Err(LeaseError::Held(record));
        }
        let previous = match self.read_record(key) {
            Ok(previous) => previous,
            Err(error) => {
                let _ = FileExt::unlock(&file);
                return Err(error.into());
            }
        };
        let unclean = previous.as_ref().is_some_and(|record| !record.released);
        let epoch = match previous {
            None => 1,
            Some(record) => match record.epoch.checked_add(1) {
                Some(next) => next,
                None => {
                    let _ = FileExt::unlock(&file);
                    return Err(StorageError::conflict(format!(
                        "lease {key} epoch space exhausted"
                    ))
                    .into());
                }
            },
        };
        let record = LeaseRecord {
            owner: self.node.clone(),
            endpoint: self.endpoint.clone(),
            epoch,
            expires_at_ms: u64::MAX,
            released: false,
        };
        if let Err(error) = self.write_record(key, &record) {
            let _ = FileExt::unlock(&file);
            return Err(error.into());
        }
        tracing::debug!(
            target: "openhuman::storage::lease",
            key,
            node = %self.node,
            epoch,
            previous_unclean = unclean,
            "[lease] local acquired"
        );
        held.insert(
            key.to_string(),
            Holding {
                file,
                record,
                version: Version::FIRST,
            },
        );
        Ok(LeaseGrant {
            key: key.to_string(),
            epoch,
            version: Version::FIRST,
            expires_at_ms: u64::MAX,
            previous_unclean: unclean,
        })
    }

    async fn renew(&self, grant: &LeaseGrant, _now_ms: u64) -> Result<LeaseGrant, LeaseError> {
        self.with_current(grant, |held| {
            let holding = held.get_mut(&grant.key).ok_or(LeaseError::Lost)?;
            holding.version = holding
                .version
                .next()
                .ok_or_else(|| StorageError::backend("lease version space exhausted"))?;
            Ok(LeaseGrant {
                version: holding.version,
                ..grant.clone()
            })
        })
    }

    async fn release(&self, grant: LeaseGrant) -> Result<(), LeaseError> {
        self.with_current(&grant, |held| {
            let holding = held.get(&grant.key).ok_or(LeaseError::Lost)?;
            let record = LeaseRecord {
                released: true,
                ..holding.record.clone()
            };
            // Persist first: on failure the lock and holding stay, so no one
            // can take the key over a record still saying "not released".
            self.write_record(&grant.key, &record)?;
            if let Some(holding) = held.remove(&grant.key) {
                let _ = FileExt::unlock(&holding.file);
            }
            tracing::debug!(
                target: "openhuman::storage::lease",
                key = %grant.key,
                node = %self.node,
                epoch = grant.epoch,
                "[lease] local released"
            );
            Ok(())
        })
    }

    async fn holder(&self, key: &str) -> Result<Option<LeaseRecord>, LeaseError> {
        validate_key(key)?;
        if let Some(holding) = self.held().get(key) {
            return Ok(Some(holding.record.clone()));
        }
        let Some(mut record) = self.read_record(key)? else {
            return Ok(None);
        };
        if !record.released {
            // A free lock under an unreleased record: its holder died, so
            // the record no longer excludes anyone.
            let file = self.open_lock(key)?;
            if file.try_lock_exclusive().is_ok() {
                let _ = FileExt::unlock(&file);
                record.expires_at_ms = 0;
            }
        }
        Ok(Some(record))
    }

    async fn holder_readonly(&self, key: &str) -> Result<Option<LeaseRecord>, LeaseError> {
        validate_key(key)?;
        if let Some(holding) = self.held().get(key) {
            return Ok(Some(holding.record.clone()));
        }
        let Some(mut record) = self.read_record(key)? else {
            return Ok(None);
        };
        if !record.released {
            let path = local_dir(&self.root, key).join(LOCK_FILE);
            match OpenOptions::new().read(true).open(&path) {
                Ok(file) => match file.try_lock_exclusive() {
                    Ok(()) => {
                        let _ = FileExt::unlock(&file);
                        record.expires_at_ms = 0;
                    }
                    Err(error) if is_contended(&error) => {}
                    Err(error) => return Err(io_error("open", &path, &error).into()),
                },
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    record.expires_at_ms = 0;
                }
                Err(error) => return Err(io_error("open", &path, &error).into()),
            }
        }
        Ok(Some(record))
    }
}

#[cfg(test)]
#[path = "lease_local_tests.rs"]
mod tests;
