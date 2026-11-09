//! Machine-local mtime state for the vault file watcher.
//!
//! The rows contain absolute paths and mtimes observed by this machine, so
//! they use a dedicated local SQLite backend opened through the storage
//! facade rather than the process-wide backend (which may be shared across
//! hosts). The legacy SQLite table is read only by the host migration path.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tinystoragedrivers::{StorageBackend, StorageConfig};
use tinystoragedrivers_api::KvStore;
use tinystoragedrivers_types::{Cursor, Precondition};

const KEY_PREFIX: &str = "openhuman.vault-watcher.";
const PAGE_SIZE: usize = 256;

#[derive(Clone)]
pub struct WatcherStateStore {
    kv: Arc<dyn KvStore>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileState {
    pub path: PathBuf,
    pub mtime_secs: u64,
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredFileState {
    path: String,
    mtime_secs: u64,
    deleted: bool,
}

impl WatcherStateStore {
    /// Open the machine-local store at `db_path` through the TSD facade.
    pub fn open(db_path: &Path) -> Result<Self> {
        let url = format!("sqlite:{}", db_path.display());
        let config = StorageConfig::parse(&url).context("parse watcher storage URL")?;
        let backend =
            crate::storage::block_on(async move { tinystoragedrivers::open(&config).await })
                .context("open watcher storage backend")?;
        Self::from_backend(backend)
    }

    fn from_backend(backend: Arc<dyn StorageBackend>) -> Result<Self> {
        let storage = backend
            .for_scope(&crate::storage::Scope::local())
            .map_err(anyhow::Error::from)
            .context("open watcher scope")?;
        Ok(Self {
            kv: Arc::clone(storage.kv()),
        })
    }

    /// Upsert the observed mtime for `path`, marking it as present.
    pub fn record_seen(&mut self, path: &Path, mtime_secs: u64) -> Result<()> {
        let path = path.to_string_lossy().into_owned();
        let key = key_for(&path);
        let value = serde_json::to_value(StoredFileState {
            path,
            mtime_secs,
            deleted: false,
        })?;
        let kv = Arc::clone(&self.kv);
        crate::storage::block_on(async move { kv.put(&key, value).await.map(|_| ()) })
            .context("write watcher state")?;
        Ok(())
    }

    /// Mark `path` deleted, retaining its prior mtime if one exists.
    pub fn record_deleted(&mut self, path: &Path) -> Result<()> {
        let path = path.to_string_lossy().into_owned();
        let key = key_for(&path);
        let kv = Arc::clone(&self.kv);
        crate::storage::block_on_anyhow(async move {
            loop {
                let current = kv.get(&key).await?;
                let (mtime_secs, expected) = match current {
                    Some(current) => {
                        let state = decode_state(&current.doc)?;
                        ensure_path(&state, &path)?;
                        (state.mtime_secs, current.unchanged())
                    }
                    None => (0, Precondition::Absent),
                };
                let value = serde_json::to_value(StoredFileState {
                    path: path.clone(),
                    mtime_secs,
                    deleted: true,
                })?;
                match kv.cas(&key, value, expected).await {
                    Ok(_) => return Ok(()),
                    Err(error) if error.kind() == tinystoragedrivers_types::ErrorKind::Conflict => {
                        continue;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        })
        .context("write deleted watcher state")?;
        Ok(())
    }

    /// Return the stored mtime for `path`, or `None` if absent or deleted.
    pub fn last_mtime(&self, path: &Path) -> Result<Option<u64>> {
        let path = path.to_string_lossy().into_owned();
        let key = key_for(&path);
        let kv = Arc::clone(&self.kv);
        let value = crate::storage::block_on(async move { kv.get(&key).await })
            .context("read watcher state")?;
        let Some(value) = value else {
            return Ok(None);
        };
        let state = decode_state(&value.doc)?;
        ensure_path(&state, &path)?;
        Ok((!state.deleted).then_some(state.mtime_secs))
    }

    /// Load every non-deleted state in stable key order.
    pub fn load_all(&self) -> Result<Vec<FileState>> {
        let kv = Arc::clone(&self.kv);
        crate::storage::block_on_anyhow(async move {
            let mut cursor: Option<Cursor> = None;
            let mut states = Vec::new();
            loop {
                let page = kv.list(KEY_PREFIX, cursor, PAGE_SIZE).await?;
                for item in page.items {
                    let state = decode_state(&item.doc)?;
                    if item.id != key_for(&state.path) {
                        return Err(anyhow!("watcher state key identity mismatch"));
                    }
                    if !state.deleted {
                        states.push(FileState {
                            path: PathBuf::from(state.path),
                            mtime_secs: state.mtime_secs,
                            deleted: false,
                        });
                    }
                }
                let Some(next) = page.next else { break };
                cursor = Some(next);
            }
            Ok(states)
        })
        .context("list watcher state")
    }
}

fn key_for(path: &str) -> String {
    let digest = Sha256::digest(path.as_bytes());
    format!("{KEY_PREFIX}{}", hex::encode(digest))
}

fn decode_state(value: &Value) -> Result<StoredFileState> {
    serde_json::from_value(value.clone()).context("decode watcher state")
}

fn ensure_path(state: &StoredFileState, path: &str) -> Result<()> {
    if state.path != path {
        return Err(anyhow!("watcher state key identity mismatch"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
