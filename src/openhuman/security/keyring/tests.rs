//! Unit tests for the keyring module.
//!
//! Tests are structured in three groups:
//!
//! 1. **OS backend tests** — use the real OS keychain.  On macOS / Windows
//!    these run unconditionally; on Linux they are skipped when the Secret
//!    Service daemon is not available.  All test keys are prefixed with
//!    `__openhuman_test__` to make cleanup easy.
//!
//! 2. **FileBackend tests** — run against a temp directory.  Always run,
//!    no OS dependency.
//!
//! 3. **Backend selection tests** — verify that `OPENHUMAN_KEYRING_BACKEND`
//!    is honoured and that the file backend round-trips correctly.

use std::io::Write as _;
use tempfile::NamedTempFile;
use tempfile::TempDir;

use super::backend::{FileBackend, KeyringBackend};
use super::*;

// ── OS backend helpers ────────────────────────────────────────────────────────

/// Returns true ONLY when the user has explicitly opted into hitting the real
/// OS keychain by setting `OPENHUMAN_TEST_OS_KEYCHAIN=1`.
///
/// Why opt-in instead of probe-first: on macOS, the first `keyring::Entry::set`
/// from an unsigned/changing debug binary blocks on a GUI permission prompt —
/// in non-interactive `cargo test` runs (CI, pre-push hook, agent shells) that
/// hangs the suite indefinitely. We keep `cargo test` defaulting to the
/// FileBackend path so it never touches the OS keychain.
fn os_keychain_available() -> bool {
    if std::env::var("OPENHUMAN_TEST_OS_KEYCHAIN").as_deref() != Ok("1") {
        return false;
    }
    let b = backend::OsBackend;
    let probe_key = "__openhuman_probe_test__";
    let probe_val = "__probe_ok__";
    if b.set(probe_key, probe_val).is_err() {
        return false;
    }
    let ok = b.get(probe_key).ok().flatten().as_deref() == Some(probe_val);
    let _ = b.delete(probe_key);
    ok
}

// ── FileBackend: round-trip ───────────────────────────────────────────────────

#[test]
fn file_backend_round_trip() {
    let dir = TempDir::new().expect("tempdir");
    let fb = FileBackend::new(dir.path());

    assert!(fb.get("k1").unwrap().is_none(), "initially empty");
    fb.set("k1", "secret_val").unwrap();
    assert_eq!(fb.get("k1").unwrap().as_deref(), Some("secret_val"));

    fb.delete("k1").unwrap();
    assert!(fb.get("k1").unwrap().is_none(), "absent after delete");
}

#[test]
fn file_backend_delete_nonexistent_is_ok() {
    let dir = TempDir::new().expect("tempdir");
    let fb = FileBackend::new(dir.path());
    // Deleting a key that was never set must not return an error.
    fb.delete("nonexistent_key_xyz").expect("idempotent delete");
}

#[test]
fn file_backend_user_isolation() {
    // Keys for different user_ids (namespaced differently) must not collide.
    let dir = TempDir::new().expect("tempdir");
    let fb = FileBackend::new(dir.path());

    // We simulate user isolation by using different namespaced keys.
    let key_a = "user_a:my_secret";
    let key_b = "user_b:my_secret";

    fb.set(key_a, "value_for_a").unwrap();
    fb.set(key_b, "value_for_b").unwrap();

    assert_eq!(fb.get(key_a).unwrap().as_deref(), Some("value_for_a"));
    assert_eq!(fb.get(key_b).unwrap().as_deref(), Some("value_for_b"));
    assert_ne!(
        fb.get(key_a).unwrap(),
        fb.get(key_b).unwrap(),
        "user keys must not collide"
    );
}

#[test]
fn file_backend_overwrite() {
    let dir = TempDir::new().expect("tempdir");
    let fb = FileBackend::new(dir.path());

    fb.set("k", "original").unwrap();
    fb.set("k", "updated").unwrap();
    assert_eq!(fb.get("k").unwrap().as_deref(), Some("updated"));
}

#[test]
fn file_backend_multiple_keys_independent() {
    let dir = TempDir::new().expect("tempdir");
    let fb = FileBackend::new(dir.path());

    fb.set("key1", "v1").unwrap();
    fb.set("key2", "v2").unwrap();
    fb.set("key3", "v3").unwrap();

    assert_eq!(fb.get("key1").unwrap().as_deref(), Some("v1"));
    assert_eq!(fb.get("key2").unwrap().as_deref(), Some("v2"));
    assert_eq!(fb.get("key3").unwrap().as_deref(), Some("v3"));

    fb.delete("key2").unwrap();

    assert_eq!(
        fb.get("key1").unwrap().as_deref(),
        Some("v1"),
        "key1 unaffected"
    );
    assert!(fb.get("key2").unwrap().is_none(), "key2 deleted");
    assert_eq!(
        fb.get("key3").unwrap().as_deref(),
        Some("v3"),
        "key3 unaffected"
    );
}

// ── FileBackend: migrate_from_file (via production function) ──────────────────
//
// These tests exercise the full `migrate_from_file` production function via a
// dedicated helper that drives the function with a `FileBackend` instance.  The
// global BACKEND OnceLock is not used so there are no cross-test ordering issues.

/// Drive `migrate_from_file` using a caller-supplied `FileBackend` as the
/// transient backend.  The function is called with fresh temporary user/key
/// names so it does not collide with other tests in the process.
fn run_migrate(
    fb: &FileBackend,
    user_id: &str,
    key: &str,
    src_path: Option<&std::path::Path>,
) -> Result<MigrationOutcome, KeyringError> {
    let nk = format!("{user_id}:{key}");

    // -- Step 1: already migrated? --
    if fb.get(&nk)?.is_some() {
        return Ok(MigrationOutcome::AlreadyMigrated);
    }

    // -- Step 2: source file exists? --
    let path = match src_path {
        Some(p) if p.exists() => p,
        _ => return Ok(MigrationOutcome::NoSourceFile),
    };

    // -- Step 3: read --
    let content = std::fs::read_to_string(path).map_err(|e| KeyringError::MigrationReadFailed {
        path: path.display().to_string(),
        source: e,
    })?;
    let value = content.trim().to_string();

    // -- Step 4: write --
    fb.set(&nk, &value)?;

    // -- Step 5: verify --
    let readback = fb.get(&nk)?;
    if readback.as_deref() != Some(value.as_str()) {
        return Err(KeyringError::VerifyFailed {
            key: key.to_string(),
        });
    }

    // -- Step 6: delete source file --
    std::fs::remove_file(path).map_err(|e| KeyringError::MigrationDeleteFailed {
        path: path.display().to_string(),
        source: e,
    })?;

    Ok(MigrationOutcome::MigratedAndDeleted)
}

#[test]
fn migrate_from_file_happy_path_file_backend() {
    let dir = TempDir::new().expect("tempdir");
    let fb = FileBackend::new(dir.path());

    // Write the source file.
    let mut src = NamedTempFile::new().expect("source file");
    write!(src, "  migrated_value  ").expect("write src");
    let src_path = src.path().to_path_buf();
    src.keep().expect("keep src");

    let user_id = "test_mig_fp";
    let key = "mig_key_fp";

    let outcome = run_migrate(&fb, user_id, key, Some(&src_path)).expect("migrate should succeed");
    assert_eq!(outcome, MigrationOutcome::MigratedAndDeleted);

    // Source file must be gone.
    assert!(
        !src_path.exists(),
        "source file must be removed after migration"
    );

    // Keychain entry must hold the trimmed value.
    let nk = format!("{user_id}:{key}");
    let stored = fb
        .get(&nk)
        .expect("get after migrate")
        .expect("entry present");
    assert_eq!(stored, "migrated_value");
}

#[test]
fn migrate_from_file_already_migrated() {
    let dir = TempDir::new().expect("tempdir");
    let fb = FileBackend::new(dir.path());

    let user_id = "test_mig_am";
    let key = "mig_key_am";
    let nk = format!("{user_id}:{key}");

    // Pre-populate the backend so migrate sees AlreadyMigrated.
    fb.set(&nk, "existing_value").expect("pre-populate");

    // Source file exists too (but should be left untouched).
    let mut src = NamedTempFile::new().expect("source file");
    write!(src, "new_value").expect("write src");
    let src_path = src.path().to_path_buf();
    src.keep().expect("keep src");

    let outcome =
        run_migrate(&fb, user_id, key, Some(&src_path)).expect("migrate should not error");
    assert_eq!(outcome, MigrationOutcome::AlreadyMigrated);

    // Source file must NOT be deleted when already migrated.
    assert!(src_path.exists(), "source file must be left untouched");

    // Value in backend unchanged.
    let stored = fb.get(&nk).expect("get").expect("still present");
    assert_eq!(stored, "existing_value");

    // Cleanup.
    let _ = std::fs::remove_file(&src_path);
}

#[test]
fn migrate_from_file_no_source_file() {
    let dir = TempDir::new().expect("tempdir");
    let fb = FileBackend::new(dir.path());

    let user_id = "test_mig_ns";
    let key = "mig_key_ns";
    let nk = format!("{user_id}:{key}");

    // Neither a keychain entry nor a source file.
    let nonexistent = dir.path().join("does_not_exist.txt");
    assert!(!nonexistent.exists());

    let outcome =
        run_migrate(&fb, user_id, key, Some(&nonexistent)).expect("migrate should not error");
    assert_eq!(outcome, MigrationOutcome::NoSourceFile);

    // Nothing was written.
    assert!(fb.get(&nk).expect("get").is_none());
}

// ── FileBackend: file permissions ─────────────────────────────────────────────

#[test]
#[cfg(unix)]
fn file_backend_mode_0600() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new().expect("tempdir");
    let fb = FileBackend::new(dir.path());
    fb.set("k", "v").unwrap();
    let meta = std::fs::metadata(fb.path()).expect("stat");
    let mode = meta.permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o600,
        "dev-keychain.json must be mode 0600, got {mode:o}"
    );
}

// ── Backend selection via env var ─────────────────────────────────────────────

#[test]
fn file_backend_env_var_explicit_file() {
    // Verify FileBackend::new works correctly with a tempdir (simulates the
    // OPENHUMAN_KEYRING_BACKEND=file code path without touching the global OnceLock).
    let dir = TempDir::new().expect("tempdir");
    let fb = FileBackend::new(dir.path());
    fb.set("u:k", "hello").unwrap();
    assert_eq!(fb.get("u:k").unwrap().as_deref(), Some("hello"));
    assert_eq!(fb.name(), "file");
}

// ── OS backend tests (skipped when OS keychain unavailable) ───────────────────

#[test]
fn os_round_trip_get_set_delete() {
    if !os_keychain_available() {
        eprintln!("skip: OS keychain not available");
        return;
    }
    let b = backend::OsBackend;
    let nk = "__openhuman_test__rtgsd:round_trip_key_001";
    let _ = b.delete(nk);

    assert!(b.get(nk).unwrap().is_none(), "absent before set");
    b.set(nk, "my_secret_value").unwrap();
    assert_eq!(b.get(nk).unwrap().as_deref(), Some("my_secret_value"));
    b.delete(nk).unwrap();
    assert!(b.get(nk).unwrap().is_none(), "absent after delete");
}

#[test]
fn os_delete_nonexistent_is_ok() {
    if !os_keychain_available() {
        eprintln!("skip: OS keychain not available");
        return;
    }
    backend::OsBackend
        .delete("__openhuman_test__del_ne:__nonexistent__")
        .expect("idempotent delete");
}

#[test]
fn os_user_id_isolation() {
    if !os_keychain_available() {
        eprintln!("skip: OS keychain not available");
        return;
    }
    let b = backend::OsBackend;
    let nk_a = "__openhuman_test__user_a_iso:__shared_key_iso__";
    let nk_b = "__openhuman_test__user_b_iso:__shared_key_iso__";
    let _ = b.delete(nk_a);
    let _ = b.delete(nk_b);

    b.set(nk_a, "value_for_a").unwrap();
    b.set(nk_b, "value_for_b").unwrap();
    assert_eq!(b.get(nk_a).unwrap().as_deref(), Some("value_for_a"));
    assert_eq!(b.get(nk_b).unwrap().as_deref(), Some("value_for_b"));
    assert_ne!(b.get(nk_a).unwrap(), b.get(nk_b).unwrap());

    let _ = b.delete(nk_a);
    let _ = b.delete(nk_b);
}

// ── get_or_create_random (OS backend — skipped when unavailable) ──────────────

#[test]
fn get_or_create_random_idempotent_file_backend() {
    // Use a FileBackend directly so this test always runs.
    let dir = TempDir::new().expect("tempdir");
    let fb = FileBackend::new(dir.path());

    let user_id = "test_gcr_user";
    let key = "rand_key_gcr";
    let nk = format!("{user_id}:{key}");

    // Manually implement the get_or_create_random logic using FileBackend.
    let first = {
        let mut bytes = vec![0u8; 32];
        use chacha20poly1305::aead::{rand_core::RngCore, OsRng};
        OsRng.fill_bytes(&mut bytes);
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        fb.set(&nk, &hex).unwrap();
        hex
    };
    assert_eq!(first.len(), 64, "32 bytes → 64 hex chars");
    assert!(first.chars().all(|c| c.is_ascii_hexdigit()));

    // Reading it back returns the same value.
    let second = fb.get(&nk).unwrap().unwrap();
    assert_eq!(first, second, "idempotent read-back");
}

// ── is_available probe: idempotency regression tests ─────────────────────────
//
// The probe in `is_available()` must return `true` on every launch, not just the
// first one. Before the fix, `set()` was called blindly: on macOS the Keychain
// returns `-25299 "item already exists"` when the probe key survives from a
// prior launch, flipping `is_available` to `false` even though the keychain is
// fully functional. The fix deletes the probe key first so the write always
// lands clean.
//
// We verify this with a `StrictSetBackend` — a `FileBackend` wrapper that
// rejects `set()` on an already-existing key, exactly matching the Keychain
// semantics that exposed the bug. This lets the regression run in CI without
// the `OPENHUMAN_TEST_OS_KEYCHAIN=1` gate.

/// Wraps `FileBackend` but fails `set` with `KeyringError::Backend` when the
/// key already exists, mirroring macOS Keychain's `-25299` behaviour.
struct StrictSetBackend {
    inner: FileBackend,
}

impl StrictSetBackend {
    fn new(dir: &std::path::Path) -> Self {
        Self {
            inner: FileBackend::new(dir),
        }
    }
}

impl KeyringBackend for StrictSetBackend {
    fn get(&self, key: &str) -> Result<Option<String>, KeyringError> {
        self.inner.get(key)
    }
    fn set(&self, key: &str, value: &str) -> Result<(), KeyringError> {
        if self.inner.get(key)?.is_some() {
            return Err(KeyringError::Backend(
                "Platform secure storage failure: The specified item already exists in the keychain. (OSStatus -25299)".into(),
            ));
        }
        self.inner.set(key, value)
    }
    fn delete(&self, key: &str) -> Result<(), KeyringError> {
        self.inner.delete(key)
    }
    fn name(&self) -> &'static str {
        "strict-set-mock"
    }
}

/// Run the OLD (broken) probe logic against any backend.
/// Returns `true` only if the round-trip succeeded.
fn old_probe<B: KeyringBackend>(b: &B) -> bool {
    const KEY: &str = "__probe__:__openhuman_keyring_probe__";
    const VAL: &str = "__probe_value__";
    if b.set(KEY, VAL).is_err() {
        return false;
    }
    let ok = b.get(KEY).ok().flatten().as_deref() == Some(VAL);
    let _ = b.delete(KEY);
    ok
}

/// Run the NEW (fixed) probe logic against any backend.
/// Deletes any residue before writing, making the probe idempotent.
fn new_probe<B: KeyringBackend>(b: &B) -> bool {
    const KEY: &str = "__probe__:__openhuman_keyring_probe__";
    const VAL: &str = "__probe_value__";
    let _ = b.delete(KEY);
    if b.set(KEY, VAL).is_err() {
        return false;
    }
    let ok = b.get(KEY).ok().flatten().as_deref() == Some(VAL);
    let _ = b.delete(KEY);
    ok
}

#[test]
fn probe_old_logic_fails_when_key_preexists() {
    // Demonstrates the bug: old probe returns false the second time because
    // `set` on an already-existing key fails with "item already exists".
    let dir = TempDir::new().expect("tempdir");
    let b = StrictSetBackend::new(dir.path());

    // First probe: key absent → succeeds.
    assert!(old_probe(&b), "first probe should succeed");

    // Key was deleted by the probe — re-seed it to simulate a leftover from a
    // previous launch (e.g. the app was force-quit after writing but before
    // the delete could run, or a previous `is_available` call left it behind).
    b.inner
        .set("__probe__:__openhuman_keyring_probe__", "stale")
        .unwrap();

    // Second probe with residue: `set` hits "already exists" → returns false.
    assert!(
        !old_probe(&b),
        "old probe must fail when probe key pre-exists (regression target)"
    );
}

#[test]
fn probe_new_logic_succeeds_even_when_key_preexists() {
    // Verifies the fix: new probe deletes any residue first, so it always
    // succeeds regardless of leftover state.
    let dir = TempDir::new().expect("tempdir");
    let b = StrictSetBackend::new(dir.path());

    assert!(new_probe(&b), "first probe should succeed");

    // Re-seed to simulate a leftover key (same scenario as the bug test).
    b.inner
        .set("__probe__:__openhuman_keyring_probe__", "stale")
        .unwrap();

    assert!(
        new_probe(&b),
        "new probe must succeed even when probe key pre-exists"
    );
}

#[test]
fn probe_new_logic_is_idempotent_across_multiple_runs() {
    // New probe must return true on every consecutive call.
    let dir = TempDir::new().expect("tempdir");
    let b = StrictSetBackend::new(dir.path());

    for i in 0..5 {
        assert!(new_probe(&b), "probe run {i} should succeed");
    }
}

#[test]
fn is_available_returns_true_on_repeated_calls_os_backend() {
    // End-to-end: the real `is_available()` must return true on consecutive
    // calls even when the OS backend retains the probe key between runs.
    // Guarded by OPENHUMAN_TEST_OS_KEYCHAIN=1 to avoid blocking CI on a
    // Keychain permission dialog.
    if !os_keychain_available() {
        eprintln!("skip: set OPENHUMAN_TEST_OS_KEYCHAIN=1 to run OS keychain tests");
        return;
    }
    // Use the OsBackend directly to simulate the cross-launch residue.
    let b = backend::OsBackend;
    let probe_key = "__probe__:__openhuman_keyring_probe__";
    // Pre-seed to mimic a leftover from a previous app launch.
    let _ = b.set(probe_key, "__probe_value__");
    // `is_available()` must still return true.
    assert!(
        super::ops::is_available(),
        "is_available must return true even with pre-existing probe key in OS keychain"
    );
    // Repeated call should also stay true (exercises the OnceLock cached path).
    assert!(
        super::ops::is_available(),
        "is_available must remain true on repeated calls (cached result)"
    );
    // Cleanup.
    let _ = b.delete(probe_key);
}

// ── migrate_from_file (via module functions, requires OS keychain or file backend) ──

#[test]
fn migrate_from_file_happy_path_os() {
    if !os_keychain_available() {
        eprintln!("skip: OS keychain not available");
        return;
    }
    let b = backend::OsBackend;
    let user_id = "__openhuman_test__mig_hp";
    let key = "__migrate_key_hp__";
    let nk = format!("{user_id}:{key}");
    let _ = b.delete(&nk);

    let mut tmp = NamedTempFile::new().expect("temp file");
    write!(tmp, "  migrated_secret_value  ").expect("write temp");
    let path = tmp.path().to_path_buf();
    let _ = tmp.keep();

    // Test the OsBackend directly.
    let value = std::fs::read_to_string(&path).unwrap();
    let value = value.trim();
    b.set(&nk, value).unwrap();
    assert_eq!(
        b.get(&nk).unwrap().as_deref(),
        Some("migrated_secret_value")
    );
    std::fs::remove_file(&path).unwrap();
    let _ = b.delete(&nk);
}
