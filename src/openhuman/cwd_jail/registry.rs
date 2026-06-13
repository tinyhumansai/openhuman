//! Jail registry — manage many jailed workspaces side-by-side.
//!
//! A [`JailRegistry`] is rooted at a single base directory (typically
//! `~/.openhuman/jails/` or `<workspace>/jails/`) and owns every active
//! jail underneath it. Each jail has:
//!
//! - A stable **id** (UUID-ish, used in paths and the index).
//! - A user-visible **label** (free text, displayed in UI, used for
//!   AppContainer profile derivation on Windows).
//! - A **directory** at `<base>/<id>/` that the [`crate::openhuman::cwd_jail::Jail`]
//!   is rooted in.
//! - **Metadata**: created/updated timestamps, backend used at create
//!   time, optional notes.
//!
//! All metadata is persisted to `<base>/index.json`. The on-disk index is
//! the source of truth; the in-memory state is rebuilt from it on every
//! [`JailRegistry::open`].
//!
//! Concurrency: a `std::sync::Mutex` guards mutations. The index file
//! is rewritten atomically via write-temp + rename. This is sufficient
//! for the single-process core; if we ever want multi-process registry
//! access we'll need OS-level file locking — explicit non-goal for now.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::jail::{Jail, JailBackend};
use super::{default_backend, spawn_with};

/// Metadata persisted for each jail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JailRecord {
    pub id: String,
    pub label: String,
    pub dir: PathBuf,
    pub backend_at_create: String,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    /// id → record. BTreeMap so list() is deterministically ordered.
    records: BTreeMap<String, JailRecord>,
    #[serde(default)]
    schema_version: u32,
}

const INDEX_SCHEMA_VERSION: u32 = 1;
const INDEX_FILENAME: &str = "index.json";

/// Top-level manager for multiple jailed workspaces.
#[derive(Debug)]
pub struct JailRegistry {
    base: PathBuf,
    index: Mutex<Index>,
}

impl JailRegistry {
    /// Open (or create) a registry rooted at `base`. The directory is
    /// created if it does not exist; the index file is loaded if present
    /// and seeded blank otherwise.
    pub fn open(base: impl AsRef<Path>) -> io::Result<Self> {
        let base = base.as_ref().to_path_buf();
        fs::create_dir_all(&base)?;
        let idx_path = base.join(INDEX_FILENAME);
        let index = if idx_path.exists() {
            log::debug!(
                "[cwd_jail] registry.open loading index {}",
                idx_path.display()
            );
            let raw = fs::read(&idx_path)?;
            serde_json::from_slice::<Index>(&raw)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
        } else {
            log::debug!("[cwd_jail] registry.open fresh index at {}", base.display());
            Index {
                records: BTreeMap::new(),
                schema_version: INDEX_SCHEMA_VERSION,
            }
        };
        log::debug!(
            "[cwd_jail] registry.open base={} records={}",
            base.display(),
            index.records.len()
        );
        Ok(Self {
            base,
            index: Mutex::new(index),
        })
    }

    /// Root directory of this registry.
    pub fn base(&self) -> &Path {
        &self.base
    }

    /// Create a new jail directory. Returns the persisted record.
    pub fn create(&self, label: impl Into<String>) -> io::Result<JailRecord> {
        let label = label.into();
        // Label is free-form user input — log only its length so we get
        // a useful breadcrumb without leaking arbitrary text into logs.
        log::debug!("[cwd_jail] registry.create label_len={}", label.len());

        // Loop until we find an id not already in the index. After a
        // process restart in the same second, the time+counter id can
        // repeat — without this loop we would silently overwrite an
        // existing record.
        let mut idx = self.index.lock().unwrap();
        let (id, dir) = loop {
            let candidate = generate_id();
            if !idx.records.contains_key(&candidate) {
                let dir = self.base.join(&candidate);
                fs::create_dir_all(&dir)?;
                break (candidate, dir);
            }
            log::trace!("[cwd_jail] id collision, regenerating");
        };

        let now = now_unix();
        let record = JailRecord {
            id: id.clone(),
            label,
            dir,
            backend_at_create: default_backend().name().to_string(),
            created_at_unix: now,
            updated_at_unix: now,
            notes: None,
        };
        idx.records.insert(id.clone(), record.clone());
        // Roll back both the in-memory insert and the freshly-created
        // directory if persistence fails — otherwise the registry would
        // expose a record through get()/list() that does not exist on
        // disk, and a subsequent reopen would silently lose it.
        if let Err(e) = self.persist(&idx) {
            idx.records.remove(&id);
            let _ = fs::remove_dir_all(&record.dir);
            log::warn!("[cwd_jail] registry.create persist failed; rolled back: {e}");
            return Err(e);
        }
        log::debug!(
            "[cwd_jail] registry.create id={id} dir={}",
            record.dir.display()
        );
        Ok(record)
    }

    /// Look up a jail by id.
    pub fn get(&self, id: &str) -> Option<JailRecord> {
        self.index.lock().unwrap().records.get(id).cloned()
    }

    /// List every active jail. Deterministic order (by id).
    pub fn list(&self) -> Vec<JailRecord> {
        self.index
            .lock()
            .unwrap()
            .records
            .values()
            .cloned()
            .collect()
    }

    /// Search by label substring (case-insensitive). Useful for UI.
    pub fn find_by_label(&self, needle: &str) -> Vec<JailRecord> {
        let needle = needle.to_lowercase();
        self.index
            .lock()
            .unwrap()
            .records
            .values()
            .filter(|r| r.label.to_lowercase().contains(&needle))
            .cloned()
            .collect()
    }

    /// Rename: changes the *label* only. The directory id stays put so
    /// existing path references keep working. AppContainer profile names
    /// are derived from `id` (stable), not `label`, for the same reason.
    pub fn rename(&self, id: &str, new_label: impl Into<String>) -> io::Result<JailRecord> {
        let new_label = new_label.into();
        log::debug!(
            "[cwd_jail] registry.rename id={id} new_label_len={}",
            new_label.len()
        );
        let mut idx = self.index.lock().unwrap();
        let record = idx
            .records
            .get_mut(id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no jail {id}")))?;
        // Snapshot the prior label/timestamp so we can revert on
        // persist failure — without that the in-memory record would
        // diverge from disk.
        let prev_label = std::mem::replace(&mut record.label, new_label);
        let prev_updated = std::mem::replace(&mut record.updated_at_unix, now_unix());
        let cloned = record.clone();
        if let Err(e) = self.persist(&idx) {
            if let Some(r) = idx.records.get_mut(id) {
                r.label = prev_label;
                r.updated_at_unix = prev_updated;
            }
            log::warn!("[cwd_jail] registry.rename persist failed; rolled back: {e}");
            return Err(e);
        }
        Ok(cloned)
    }

    /// Update the free-form notes field.
    pub fn set_notes(&self, id: &str, notes: Option<String>) -> io::Result<JailRecord> {
        log::debug!(
            "[cwd_jail] registry.set_notes id={id} has_notes={}",
            notes.is_some()
        );
        let mut idx = self.index.lock().unwrap();
        let record = idx
            .records
            .get_mut(id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no jail {id}")))?;
        let prev_notes = std::mem::replace(&mut record.notes, notes);
        let prev_updated = std::mem::replace(&mut record.updated_at_unix, now_unix());
        let cloned = record.clone();
        if let Err(e) = self.persist(&idx) {
            if let Some(r) = idx.records.get_mut(id) {
                r.notes = prev_notes;
                r.updated_at_unix = prev_updated;
            }
            log::warn!("[cwd_jail] registry.set_notes persist failed; rolled back: {e}");
            return Err(e);
        }
        Ok(cloned)
    }

    /// Delete a jail. Removes both the directory and the index entry.
    ///
    /// Refuses to delete a jail whose directory is not under `self.base` —
    /// belt-and-suspenders against a corrupted index pointing at `/`.
    /// Disk deletion happens *before* the in-memory record is removed,
    /// so a filesystem error doesn't leave the registry in a state
    /// where the entry is gone in-memory but the directory survives on
    /// disk until the next `open()` reload.
    pub fn delete(&self, id: &str) -> io::Result<()> {
        let mut idx = self.index.lock().unwrap();
        let record = idx
            .records
            .get(id)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no jail {id}")))?;
        log::debug!(
            "[cwd_jail] registry.delete id={id} dir={}",
            record.dir.display()
        );

        let resolved = record
            .dir
            .canonicalize()
            .unwrap_or_else(|_| record.dir.clone());
        let resolved_base = self
            .base
            .canonicalize()
            .unwrap_or_else(|_| self.base.clone());
        if !resolved.starts_with(&resolved_base) {
            // Index is suspicious — don't touch anything on disk and
            // leave the in-memory record alone too. The caller can
            // diagnose and fix.
            log::warn!(
                "[cwd_jail] refusing delete: dir {} not under base {}",
                resolved.display(),
                resolved_base.display()
            );
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "refusing to delete jail outside registry base: {}",
                    resolved.display()
                ),
            ));
        }

        if record.dir.exists() {
            fs::remove_dir_all(&record.dir)?;
        }
        // Disk side succeeded — now remove from the index and persist.
        // If persist fails here the directory is already gone, so we
        // can't fully roll back; we keep the in-memory removal aligned
        // with disk reality and surface the error.
        idx.records.remove(id);
        if let Err(e) = self.persist(&idx) {
            log::warn!(
                "[cwd_jail] registry.delete persist failed after dir removal; \
                 index.json may resurrect id={id} on next reopen: {e}"
            );
            return Err(e);
        }
        Ok(())
    }

    /// Drop *all* jails. Convenience for tests / "reset everything"
    /// flows. Returns the number of jails removed.
    pub fn clear(&self) -> io::Result<usize> {
        let ids: Vec<String> = self.index.lock().unwrap().records.keys().cloned().collect();
        let n = ids.len();
        log::debug!("[cwd_jail] registry.clear dropping n={n}");
        for id in ids {
            self.delete(&id)?;
        }
        Ok(n)
    }

    /// Spawn `cmd` inside the named jail, using the default backend.
    /// Convenience wrapper — the same effect as
    /// `spawn(&Jail::new(record.dir, record.label), cmd)`.
    pub fn spawn_in(&self, id: &str, cmd: Command) -> io::Result<Child> {
        let jail = self.jail_for(id)?;
        log::debug!("[cwd_jail] registry.spawn_in id={id}");
        default_backend().spawn(&jail, cmd)
    }

    /// Same as [`spawn_in`] but with a caller-supplied backend.
    pub fn spawn_in_with(
        &self,
        id: &str,
        backend: &dyn JailBackend,
        cmd: Command,
    ) -> io::Result<Child> {
        let jail = self.jail_for(id)?;
        log::debug!(
            "[cwd_jail] registry.spawn_in_with id={id} backend={}",
            backend.name()
        );
        spawn_with(backend, &jail, cmd)
    }

    /// Build a canonicalized [`Jail`] for the given id, refusing if the
    /// persisted record points outside `self.base`. Centralizes the
    /// containment check so both `spawn_in` and `spawn_in_with` are
    /// protected against a corrupted index that could otherwise be used
    /// to bypass the directory jail root.
    fn jail_for(&self, id: &str) -> io::Result<Jail> {
        let record = self
            .get(id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no jail {id}")))?;

        let resolved = record
            .dir
            .canonicalize()
            .unwrap_or_else(|_| record.dir.clone());
        let resolved_base = self
            .base
            .canonicalize()
            .unwrap_or_else(|_| self.base.clone());
        if !resolved.starts_with(&resolved_base) {
            log::warn!(
                "[cwd_jail] refusing spawn: jail {id} dir {} not under base {}",
                resolved.display(),
                resolved_base.display()
            );
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "jail {id} dir {} is outside registry base {}",
                    resolved.display(),
                    resolved_base.display()
                ),
            ));
        }

        let mut jail = Jail::new(&record.dir, &record.label);
        jail.canonicalize()?;
        Ok(jail)
    }

    /// Atomic-rename write of the index. Falls back to direct write on
    /// Windows if rename-over fails (Windows traditionally refused
    /// rename-over-existing, though modern NTFS/Win10 supports it).
    fn persist(&self, idx: &Index) -> io::Result<()> {
        let path = self.base.join(INDEX_FILENAME);
        let tmp = self.base.join(format!("{INDEX_FILENAME}.tmp"));
        let bytes = serde_json::to_vec_pretty(idx)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        fs::write(&tmp, &bytes)?;
        match fs::rename(&tmp, &path) {
            Ok(()) => {
                log::trace!(
                    "[cwd_jail] registry.persist atomic-rename n={}",
                    idx.records.len()
                );
                Ok(())
            }
            Err(e) => {
                // Fallback: direct overwrite.
                log::debug!(
                    "[cwd_jail] registry.persist rename failed ({e}); falling back to overwrite"
                );
                fs::write(&path, &bytes)?;
                let _ = fs::remove_file(&tmp);
                Ok(())
            }
        }
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Short, URL-safe id. Not cryptographically random — we use it as a
/// directory name, not a token.
fn generate_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let ts = now_unix();
    format!("j{ts:x}{n:x}")
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
