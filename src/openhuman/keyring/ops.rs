//! Core keyring operations: get, set, delete, probe, random generation, and migration.
//!
//! All public functions delegate to the active backend selected by [`crate::openhuman::keyring::store`].

use std::path::Path;
use std::sync::OnceLock;

use chacha20poly1305::aead::{rand_core::RngCore, OsRng};

use crate::openhuman::keyring::error::KeyringError;
use crate::openhuman::keyring::store::backend;

// Cached result of the one-time keychain probe. Running the probe on every
// `is_available()` call means 4 OS-keychain round-trips (delete / set / get /
// delete) per call, which triggers repeated macOS access-permission dialogs
// and starves callers that poll frequently (e.g. snapshot, wallet guards).
// The backend selection is already frozen in a OnceLock, so the probe result
// is stable for the lifetime of the process — caching it here is safe.
static AVAILABILITY_CACHE: OnceLock<bool> = OnceLock::new();

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

/// Retrieve a secret from the active backend.
///
/// Returns `Ok(None)` when no entry exists for this user + key combination.
/// Never logs the secret value.
pub fn get(user_id: &str, key: &str) -> Result<Option<String>, KeyringError> {
    log::debug!("[keyring] get user_id={user_id} key={key}");
    let namespaced = namespaced_key(user_id, key);
    let result = backend().get(&namespaced);
    match &result {
        Ok(Some(_)) => log::debug!("[keyring] get hit user_id={user_id} key={key}"),
        Ok(None) => log::debug!("[keyring] get miss user_id={user_id} key={key}"),
        Err(e) => log::warn!(
            "[keyring] get error user_id={user_id} key={key}: {e} | detail={}",
            e.diagnostic()
        ),
    }
    result
}

/// Store a secret in the active backend.
///
/// Overwrites any existing entry for this user + key. Never logs the value.
pub fn set(user_id: &str, key: &str, value: &str) -> Result<(), KeyringError> {
    log::debug!("[keyring] set user_id={user_id} key={key}");
    let namespaced = namespaced_key(user_id, key);
    let result = backend().set(&namespaced, value);
    match &result {
        Ok(()) => log::debug!("[keyring] set ok user_id={user_id} key={key}"),
        Err(e) => log::warn!(
            "[keyring] set error user_id={user_id} key={key}: {e} | detail={}",
            e.diagnostic()
        ),
    }
    result
}

/// Delete a secret from the active backend.
///
/// Returns `Ok(())` even if no entry existed (idempotent).
pub fn delete(user_id: &str, key: &str) -> Result<(), KeyringError> {
    log::debug!("[keyring] delete user_id={user_id} key={key}");
    let namespaced = namespaced_key(user_id, key);
    let result = backend().delete(&namespaced);
    match &result {
        Ok(()) => log::debug!("[keyring] delete ok user_id={user_id} key={key}"),
        Err(e) => log::warn!(
            "[keyring] delete error user_id={user_id} key={key}: {e} | detail={}",
            e.diagnostic()
        ),
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
    *AVAILABILITY_CACHE.get_or_init(probe_availability)
}

fn probe_availability() -> bool {
    const PROBE_USER: &str = "__probe__";
    const PROBE_KEY: &str = "__openhuman_keyring_probe__";
    const PROBE_VALUE: &str = "__probe_value__";

    log::debug!(
        "[keyring] is_available probe starting backend={}",
        backend().name()
    );

    // File-based and mock backends are always available.
    let b = backend();
    if b.name() == "file" || b.name() == "mock" || b.name() == "encrypted_file" {
        log::debug!("[keyring] is_available=true (non-os backend)");
        return true;
    }

    let result = (|| -> Result<bool, KeyringError> {
        // Delete any leftover probe key from a previous launch before writing.
        // Without this, `set` fails with "item already exists" (-25299) on
        // every launch after the first, causing `is_available` to incorrectly
        // return false even when the keychain is fully functional.
        let _ = delete(PROBE_USER, PROBE_KEY);
        set(PROBE_USER, PROBE_KEY, PROBE_VALUE)?;
        let readback = get(PROBE_USER, PROBE_KEY)?;
        delete(PROBE_USER, PROBE_KEY)?;
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
) -> Result<String, KeyringError> {
    log::debug!("[keyring] get_or_create_random user_id={user_id} key={key} len_bytes={len_bytes}");

    if len_bytes == 0 {
        return Err(KeyringError::Backend(
            "get_or_create_random requires len_bytes > 0".to_string(),
        ));
    }

    if let Some(existing) = get(user_id, key)? {
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
    set(user_id, key, &hex_value)?;

    // Verify write succeeded.
    let readback = get(user_id, key)?;
    if readback.as_deref() != Some(&hex_value) {
        log::warn!(
            "[keyring] get_or_create_random write verification failed user_id={user_id} key={key}"
        );
        return Err(KeyringError::VerifyFailed {
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
/// On any failure after the file has been read but before it can be deleted,
/// the file is **not** deleted and `Err` is returned so the caller can retry.
pub fn migrate_from_file(
    user_id: &str,
    key: &str,
    path: &Path,
) -> Result<MigrationOutcome, KeyringError> {
    log::debug!(
        "[keyring] migrate_from_file user_id={user_id} key={key} path={}",
        path.display()
    );

    // Step 1: check if already migrated.
    if get(user_id, key)?.is_some() {
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

    // Step 3: read the file.
    log::debug!(
        "[keyring] migrate_from_file reading source file path={}",
        path.display()
    );
    let file_content =
        std::fs::read_to_string(path).map_err(|e| KeyringError::MigrationReadFailed {
            path: path.display().to_string(),
            source: e,
        })?;
    let value = file_content.trim().to_string();

    // Step 4: write to backend.
    log::debug!("[keyring] migrate_from_file writing to backend user_id={user_id} key={key}");
    set(user_id, key, &value)?;

    // Step 5: verify read-back matches.
    let readback = get(user_id, key)?;
    if readback.as_deref() != Some(value.as_str()) {
        log::warn!(
            "[keyring] migrate_from_file verification failed user_id={user_id} key={key}; NOT deleting source file"
        );
        return Err(KeyringError::VerifyFailed {
            key: key.to_string(),
        });
    }

    // Step 6: delete the source file (only after verified write).
    log::debug!(
        "[keyring] migrate_from_file deleting source file path={}",
        path.display()
    );
    std::fs::remove_file(path).map_err(|e| KeyringError::MigrationDeleteFailed {
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

// ── Test helpers ──────────────────────────────────────────────────────────────

/// Force-reset the backend to a custom implementation.
///
/// Only available in `#[cfg(test)]`.  Call this at the top of each test that
/// needs keyring isolation.  Panics if the backend was already initialized —
/// tests using this must run before any keyring call in the same process
/// (i.e. in a dedicated test binary or at the very start of a test).
#[cfg(test)]
pub(crate) fn force_backend_for_test(
    b: Box<dyn crate::openhuman::keyring::backend::KeyringBackend>,
) {
    use crate::openhuman::keyring::store::BACKEND;
    if BACKEND.set(b).is_err() {
        panic!("force_backend_for_test must be called before BACKEND initialization");
    }
}
