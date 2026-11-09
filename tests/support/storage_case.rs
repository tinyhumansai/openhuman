//! The configured storage backend for the `storage_*_e2e` suites and
//! `cli_storage_url_e2e`.
//!
//! These suites test the host's wiring on a configured backend (which domain
//! stores land in it, which scope they use, what a spawned core reads back),
//! not the driver. They run on SQLite, the desktop default, in a temp
//! directory. Whether each driver honours the storage ports (nested scopes,
//! compare-and-swap across processes, fencing, migrations, document secrets)
//! is proven upstream in `tinystoragedrivers`' `conformance` module, which its
//! CI runs against every driver including a MongoDB replica set.
//!
//! The backend slot is process-wide, so the tests of one binary run one at a
//! time (they share a lock) and the slot is emptied when a test ends, pass or
//! fail.
//!
//! Include with `#[path = "support/storage_case.rs"] mod storage_case;`.
//! Start an in-process test with [`Case::open`] (for an async test use
//! `#[tokio::test(flavor = "multi_thread", worker_threads = 2)]`), or
//! [`UrlCase::open`] for suites that spawn `openhuman-core` against the URL.

#![allow(dead_code)]

use std::sync::{Arc, Mutex, MutexGuard};

use openhuman_core::storage::{self, StorageBackend};

/// The slot is process-wide, so tests of one binary take turns.
static SLOT_LOCK: Mutex<()> = Mutex::new(());

fn lock_slot() -> MutexGuard<'static, ()> {
    SLOT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// One running test: a configured backend and the lock on the process-wide
/// storage slot. Dropping it empties the slot.
pub struct Case {
    /// The URL the backend was opened from.
    pub url: String,
    /// The directory the backend keeps its data in.
    pub data_dir: tempfile::TempDir,
    backend: Arc<dyn StorageBackend>,
    _lock: MutexGuard<'static, ()>,
}

impl Case {
    /// Opens a fresh backend on a new temp directory.
    ///
    /// The default is SQLite. Set `OPENHUMAN_TEST_STORAGE_BACKEND=memory` to
    /// run the same host storage end-to-end cases against TSD's memory driver.
    pub fn open() -> Case {
        let lock = lock_slot();
        let data_dir = tempfile::tempdir().expect("temp dir");
        let url = match std::env::var("OPENHUMAN_TEST_STORAGE_BACKEND").as_deref() {
            Ok("memory") => "memory".to_string(),
            Ok("sqlite") | Err(std::env::VarError::NotPresent) => {
                format!("sqlite:{}", data_dir.path().join("sqlite").display())
            }
            Ok(value) => panic!(
                "unsupported OPENHUMAN_TEST_STORAGE_BACKEND={value:?}; use `memory` or `sqlite`"
            ),
            Err(error) => panic!("read OPENHUMAN_TEST_STORAGE_BACKEND: {error}"),
        };
        let open_url = url.clone();
        // Opened on the storage bridge's runtime, the one the stores use, so
        // the backend's tasks outlive this call.
        let backend = storage::block_on(async move { storage::open(&open_url).await })
            .unwrap_or_else(|error| panic!("open the sqlite backend: {error}"));
        Case {
            url,
            data_dir,
            backend,
            _lock: lock,
        }
    }

    /// The opened backend.
    pub fn backend(&self) -> Arc<dyn StorageBackend> {
        Arc::clone(&self.backend)
    }

    /// Makes the backend the process's storage backend.
    pub fn install(&self) {
        storage::install(self.backend());
    }
}

impl Drop for Case {
    fn drop(&mut self) {
        storage::clear();
    }
}

/// A SQLite URL for suites that hand it to a spawned `openhuman-core` process
/// instead of opening the backend in the test process.
pub struct UrlCase {
    pub url: String,
    pub data_dir: tempfile::TempDir,
    _lock: MutexGuard<'static, ()>,
}

impl UrlCase {
    /// A SQLite URL on a new temp directory. Takes the same lock as
    /// [`Case`]: the spawned cores and this process share the environment.
    pub fn open() -> UrlCase {
        let lock = lock_slot();
        let data_dir = tempfile::tempdir().expect("temp dir");
        let url = format!("sqlite:{}", data_dir.path().join("sqlite").display());
        UrlCase {
            url,
            data_dir,
            _lock: lock,
        }
    }
}
