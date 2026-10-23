//! Core keyring operations: get, set, delete, probe, random generation, and migration.
//!
//! All public functions delegate to the active backend selected by [`crate::security::secret_storage::store`].

use std::path::Path;

use chacha20poly1305::aead::{rand_core::RngCore, OsRng};
use parking_lot::Mutex;

use crate::security::secret_storage::error::SecretStorageError;
use crate::security::secret_storage::store::backend;

// Cached result of the keychain probe. A single Mutex<Option<bool>> is used
// instead of a separate AtomicBool + RwLock pair to eliminate the race where
// thread A sets AVAILABILITY_PROBED=true before writing the result, causing
// thread B to read None from the cache and incorrectly return false.
//
// With a single Mutex the first thread to acquire it runs the probe and stores
// the result; all other threads block on the Mutex until the result is ready,
// then read it on the same lock acquisition.
//
// The Mutex is never held across async suspension points so contention is
// bounded to the probe duration (a single keychain round-trip on the first
// call, a pointer read on every subsequent call).
static AVAILABILITY_CACHE: Mutex<Option<bool>> = Mutex::new(None);

// ── Outcome type ─────────────────────────────────────────────────────────────

/// Outcome of a file-to-keychain migration attempt.
#[derive(Debug, PartialEq, Eq)]
pub enum MigrationOutcome {
    /// A keychain entry already existed — no action taken.
    AlreadyMigrated,
    /// Source file was read, stored in keychain, verified, and then deleted.
    MigratedAndDeleted,
    /// Source file did not exist; nothing to migrate.
    NoSourceFile,
}

// ── Core operations ───────────────────────────────────────────────────────────
//
// `get` / `set` / `delete` serve a user's secrets from the storage backend
// when the host configured one (`crate::storage::secrets`, the acting agent's
// scope), else from the process backend. The `process_*` functions always use
// the process backend: the availability probe and the app-level keys
// (`get_or_create_random`, `migrate_from_file` — the config encryption key)
// belong to this process, not to an agent.

/// The storage-backed secret store for this call, when one is configured.
fn storage_secrets(
) -> Result<Option<tinystoragedrivers::secrets::DocumentSecrets>, SecretStorageError> {
    crate::storage::secrets::current()
        .map_err(|error| SecretStorageError::Backend(format!("storage secrets: {error}")))
}

fn storage_error(error: crate::storage::StorageError) -> SecretStorageError {
    SecretStorageError::Backend(format!("storage secrets: {error}"))
}

/// Retrieve a secret: from the storage backend when one is configured,
/// else from the process backend.
///
/// Returns `Ok(None)` when no entry exists for this user + key combination.
/// Never logs the secret value.
pub fn get(user_id: &str, key: &str) -> Result<Option<String>, SecretStorageError> {
    let Some(secrets) = storage_secrets()? else {
        return process_get(user_id, key);
    };
    log::debug!("[keyring] get (storage)");
    get_adopting(secrets, &namespaced_key(user_id, key), key, || {
        legacy_or_warn(process_get(user_id, key))
    })
}

/// The legacy process-backend value to adopt. The storage backend is
/// authoritative once configured, so a failing legacy backend must not fail
/// the read; it is logged so a broken OS keychain does not look like
/// "nothing stored".
fn legacy_or_warn(result: Result<Option<String>, SecretStorageError>) -> Option<String> {
    match result {
        Ok(value) => value,
        Err(error) => {
            log::warn!("[keyring] legacy process-backend read failed during adoption: {error}");
            None
        }
    }
}

/// Read `name` from the storage-backed `secrets`. On a miss, ask `legacy` for
/// a secret an earlier process-keyring install stored and adopt it into
/// storage, so enabling a backend does not hide it. `key` is only for errors.
fn get_adopting(
    secrets: tinystoragedrivers::secrets::DocumentSecrets,
    name: &str,
    key: &str,
    legacy: impl FnOnce() -> Option<String>,
) -> Result<Option<String>, SecretStorageError> {
    use tinystoragedrivers::secrets::SecretStore as _;
    let write_back = secrets.clone();
    let owned = name.to_string();
    let value = crate::storage::block_on(async move { secrets.get(&owned).await })
        .map_err(storage_error)?;
    let Some(value) = value else {
        let legacy = legacy();
        if let Some(legacy) = &legacy {
            let owned = name.to_string();
            let bytes = zeroize::Zeroizing::new(legacy.as_bytes().to_vec());
            match crate::storage::block_on(async move { write_back.set(&owned, &bytes).await }) {
                Ok(()) => log::debug!("[keyring] adopted process-backend secret into storage"),
                Err(_) => log::warn!("[keyring] could not adopt process-backend secret"),
            }
        }
        return Ok(legacy);
    };
    String::from_utf8(value.to_vec())
        .map(Some)
        .map_err(|source| SecretStorageError::InvalidUtf8 {
            key: key.to_string(),
            source,
        })
}

/// Store a secret: in the storage backend when one is configured, else in
/// the process backend. Overwrites any existing entry. Never logs the value.
pub fn set(user_id: &str, key: &str, value: &str) -> Result<(), SecretStorageError> {
    use tinystoragedrivers::secrets::SecretStore as _;
    let Some(secrets) = storage_secrets()? else {
        return process_set(user_id, key, value);
    };
    log::debug!("[keyring] set (storage)");
    let name = namespaced_key(user_id, key);
    let value = zeroize::Zeroizing::new(value.as_bytes().to_vec());
    crate::storage::block_on(async move { secrets.set(&name, &value).await }).map_err(storage_error)
}

/// Delete a secret: from the storage backend when one is configured, else
/// from the process backend. Idempotent.
pub fn delete(user_id: &str, key: &str) -> Result<(), SecretStorageError> {
    use tinystoragedrivers::secrets::SecretStore as _;
    let Some(secrets) = storage_secrets()? else {
        return process_delete(user_id, key);
    };
    log::debug!("[keyring] delete (storage)");
    let name = namespaced_key(user_id, key);
    crate::storage::block_on(async move { secrets.delete(&name).await.map(|_| ()) })
        .map_err(storage_error)?;
    // Also drop an adoptable process-backend copy so it cannot resurface.
    if process_delete(user_id, key).is_err() {
        log::warn!("[keyring] delete: process-backend copy not removed");
    }
    Ok(())
}

// ── Process backend ───────────────────────────────────────────────────────────

/// Retrieve a secret from the process backend.
///
/// Returns `Ok(None)` when no entry exists for this user + key combination.
/// Never logs the secret value.
pub(crate) fn process_get(user_id: &str, key: &str) -> Result<Option<String>, SecretStorageError> {
    log::debug!("[keyring] get");
    let namespaced = namespaced_key(user_id, key);
    let result = backend().get(&namespaced);
    match &result {
        Ok(Some(_)) => log::debug!("[keyring] get hit"),
        Ok(None) => log::debug!("[keyring] get miss"),
        Err(e) => log::warn!("[keyring] get error: {e} | detail={}", e.diagnostic()),
    }
    result
}

/// Store a secret in the process backend.
///
/// Overwrites any existing entry for this user + key. Never logs the value.
pub(crate) fn process_set(user_id: &str, key: &str, value: &str) -> Result<(), SecretStorageError> {
    log::debug!("[keyring] set");
    let namespaced = namespaced_key(user_id, key);
    let result = backend().set(&namespaced, value);
    match &result {
        Ok(()) => log::debug!("[keyring] set ok"),
        Err(e) => log::warn!("[keyring] set error: {e} | detail={}", e.diagnostic()),
    }
    result
}

/// Delete a secret from the process backend.
///
/// Returns `Ok(())` even if no entry existed (idempotent).
pub(crate) fn process_delete(user_id: &str, key: &str) -> Result<(), SecretStorageError> {
    log::debug!("[keyring] delete");
    let namespaced = namespaced_key(user_id, key);
    let result = backend().delete(&namespaced);
    match &result {
        Ok(()) => log::debug!("[keyring] delete ok"),
        Err(e) => log::warn!("[keyring] delete error: {e} | detail={}", e.diagnostic()),
    }
    result
}

/// Probe whether the active backend is usable on this machine.
///
/// For the `file` and `mock` backends this always returns `true`.  For the
/// `os` backend on Linux headless systems (no Secret Service daemon) this
/// returns `false`; callers should fall back to file-based storage.
///
/// The result is cached after the first call — the probe runs exactly once
/// per process lifetime. This prevents repeated OS-keychain round-trips (and
/// the macOS access-permission dialogs they trigger) when polled by
/// wallet guards or snapshot loops.
pub fn is_available() -> bool {
    // Secrets on a storage backend do not depend on the OS keychain.
    if matches!(storage_secrets(), Ok(Some(_))) {
        return true;
    }
    let mut cached = AVAILABILITY_CACHE.lock();
    if let Some(val) = *cached {
        return val;
    }
    // First caller: run the probe under the lock so concurrent callers block
    // until the result is ready rather than racing on a separate atomic flag.
    let result = probe_availability();
    *cached = Some(result);
    result
}

/// Reset the cached probe result so the next [`is_available`] call re-runs
/// the OS keychain probe. Used by the retry-probe flow when the user grants
/// keychain access from Settings.
pub fn reset_availability_cache() {
    log::info!("[keyring] reset_availability_cache: clearing cached probe result");
    *AVAILABILITY_CACHE.lock() = None;
}

/// Returns the name of the active keyring backend (e.g. `"os"`, `"file"`,
/// `"encrypted_file"`).
pub fn backend_name() -> String {
    backend().name().to_string()
}

fn probe_availability() -> bool {
    const PROBE_USER: &str = "__probe__";
    const PROBE_KEY: &str = "__openhuman_keyring_probe__";
    const PROBE_VALUE: &str = "__probe_value__";

    log::debug!(
        "[keyring] is_available probe starting backend={}",
        backend().name()
    );

    let b = backend();
    if let Some(available) = backend_availability_without_probe(
        b.name(),
        b.name() == "encrypted_file" && super::encrypted_file_backend::master_key_available(),
    ) {
        log::debug!("[keyring] is_available={available} (non-os backend)");
        return available;
    }

    let result = (|| -> Result<bool, SecretStorageError> {
        // Delete any leftover probe key from a previous launch before writing.
        // Without this, `set` fails with "item already exists" (-25299) on
        // every launch after the first, causing `is_available` to incorrectly
        // return false even when the keychain is fully functional.
        let _ = process_delete(PROBE_USER, PROBE_KEY);
        process_set(PROBE_USER, PROBE_KEY, PROBE_VALUE)?;
        let readback = process_get(PROBE_USER, PROBE_KEY)?;
        process_delete(PROBE_USER, PROBE_KEY)?;
        Ok(readback.as_deref() == Some(PROBE_VALUE))
    })();

    match result {
        Ok(ok) => {
            log::debug!("[keyring] is_available={ok}");
            ok
        }
        Err(e) => {
            // Logged at warn (not debug): a failed probe flips `use_keychain`
            // off, which silently changes where auth secrets are read/written.
            // The detail captures the real cause (locked keychain / denied
            // prompt / no Secret Service) instead of just the lossy Display.
            log::warn!(
                "[keyring] is_available=false (probe failed): {e} | detail={}",
                e.diagnostic()
            );
            false
        }
    }
}

fn backend_availability_without_probe(name: &str, encrypted_key_available: bool) -> Option<bool> {
    match name {
        "memory" | "file" | "mock" => Some(true),
        "encrypted_file" => Some(encrypted_key_available),
        _ => None,
    }
}

/// Retrieve or generate-and-store a random hex secret of `len_bytes` bytes.
///
/// If an entry already exists it is returned unchanged (idempotent).
/// If no entry exists a fresh random value is generated, stored, and returned.
///
/// The returned value is a lowercase hex string of length `len_bytes * 2`.
pub fn get_or_create_random(
    user_id: &str,
    key: &str,
    len_bytes: usize,
) -> Result<String, SecretStorageError> {
    log::debug!("[keyring] get_or_create_random user_id={user_id} key={key} len_bytes={len_bytes}");

    if len_bytes == 0 {
        return Err(SecretStorageError::Backend(
            "get_or_create_random requires len_bytes > 0".to_string(),
        ));
    }

    if let Some(existing) = process_get(user_id, key)? {
        log::debug!(
            "[keyring] get_or_create_random returning existing value user_id={user_id} key={key}"
        );
        return Ok(existing);
    }

    // Generate random bytes using the OS CSPRNG.
    let mut bytes = vec![0u8; len_bytes];
    OsRng.fill_bytes(&mut bytes);
    let hex_value = hex_encode(&bytes);

    log::debug!("[keyring] get_or_create_random creating new entry user_id={user_id} key={key}");
    process_set(user_id, key, &hex_value)?;

    // Verify write succeeded.
    let readback = process_get(user_id, key)?;
    if readback.as_deref() != Some(&hex_value) {
        log::warn!(
            "[keyring] get_or_create_random write verification failed user_id={user_id} key={key}"
        );
        return Err(SecretStorageError::VerifyFailed {
            key: key.to_string(),
        });
    }

    log::debug!("[keyring] get_or_create_random created and verified user_id={user_id} key={key}");
    Ok(hex_value)
}

/// Migrate a secret from a file into the active backend.
///
/// Semantics:
/// - If an entry already exists → [`MigrationOutcome::AlreadyMigrated`].
/// - If no entry but `path` exists → read, store, verify, delete file →
///   [`MigrationOutcome::MigratedAndDeleted`].
/// - If neither exists → [`MigrationOutcome::NoSourceFile`].
///
/// Volatile backends reject migration and retain the durable source.
/// On any failure after the file has been read but before it can be deleted,
/// the file is **not** deleted and `Err` is returned so the caller can retry.
pub fn migrate_from_file(
    user_id: &str,
    key: &str,
    path: &Path,
) -> Result<MigrationOutcome, SecretStorageError> {
    log::debug!(
        "[keyring] migrate_from_file user_id={user_id} key={key} path={}",
        path.display()
    );

    // Step 1: check if already migrated.
    if process_get(user_id, key)?.is_some() {
        log::debug!("[keyring] migrate_from_file already migrated user_id={user_id} key={key}");
        return Ok(MigrationOutcome::AlreadyMigrated);
    }

    // Step 2: check if source file exists.
    if !path.exists() {
        log::debug!(
            "[keyring] migrate_from_file no source file user_id={user_id} key={key} path={}",
            path.display()
        );
        return Ok(MigrationOutcome::NoSourceFile);
    }

    if !backend().is_persistent() {
        return Err(SecretStorageError::Backend(
            "file migration requires persistent secret storage; source retained".into(),
        ));
    }

    // Step 3: read the file.
    log::debug!(
        "[keyring] migrate_from_file reading source file path={}",
        path.display()
    );
    let file_content =
        std::fs::read_to_string(path).map_err(|e| SecretStorageError::MigrationReadFailed {
            path: path.display().to_string(),
            source: e,
        })?;
    let value = file_content.trim().to_string();

    // Step 4: write to backend.
    log::debug!("[keyring] migrate_from_file writing to backend user_id={user_id} key={key}");
    process_set(user_id, key, &value)?;

    // Step 5: verify read-back matches.
    let readback = process_get(user_id, key)?;
    if readback.as_deref() != Some(value.as_str()) {
        log::warn!(
            "[keyring] migrate_from_file verification failed user_id={user_id} key={key}; NOT deleting source file"
        );
        return Err(SecretStorageError::VerifyFailed {
            key: key.to_string(),
        });
    }

    // Step 6: delete the source file (only after verified write).
    log::debug!(
        "[keyring] migrate_from_file deleting source file path={}",
        path.display()
    );
    std::fs::remove_file(path).map_err(|e| SecretStorageError::MigrationDeleteFailed {
        path: path.display().to_string(),
        source: e,
    })?;

    log::info!(
        "[keyring] migrate_from_file completed user_id={user_id} key={key} path={}",
        path.display()
    );
    Ok(MigrationOutcome::MigratedAndDeleted)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Produce the namespaced key used inside the backend store.
///
/// Format: `"{user_id}:{key}"`.  This ensures user A's keys are never
/// reachable as user B's keys.
pub(crate) fn namespaced_key(user_id: &str, key: &str) -> String {
    format!("{user_id}:{key}")
}

pub(crate) fn hex_encode(data: &[u8]) -> String {
    super::crypto::hex_encode(data)
}

#[cfg(test)]
#[path = "ops_adoption_tests.rs"]
mod adoption_tests;
