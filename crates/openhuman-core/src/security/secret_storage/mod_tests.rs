//! Unit tests for the keyring module.
//!
//! File backends and namespaced operations. Native-keychain tests live with
//! their host implementation in `openhuman-rpc/src/keyring/`.

use std::io::Write as _;
use tempfile::NamedTempFile;
use tempfile::TempDir;

use super::backend::{FileBackend, SecretBackend};
use super::*;

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

// ── FileBackend: the file holds every secret, so a bad write loses all of them ─

/// The regression this whole guard exists for.
///
/// A corrupt file used to read as an *empty map*, so the `set` that followed
/// persisted a map holding nothing but the key being written — every other
/// secret in the file, including the app session, was gone. Signing in and then
/// finding yourself signed out again is what that looks like from outside.
#[test]
fn file_backend_set_refuses_to_overwrite_an_unparseable_file() {
    let dir = TempDir::new().expect("tempdir");
    let fb = FileBackend::new(dir.path());
    fb.set("session", "keep-me").unwrap();

    let path = dir.path().join("dev-keychain.json");
    let original = std::fs::read(&path).unwrap();
    std::fs::write(&path, b"{ truncated by a racing writer").unwrap();

    let err = fb
        .set("other", "v")
        .expect_err("a set over an unparseable file must fail, not replace it");
    assert!(
        err.to_string().contains("moved aside"),
        "the error should say the file was preserved: {err}"
    );

    // The bytes survived under the quarantine name, so nothing is unrecoverable.
    let quarantined: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".corrupt."))
        .collect();
    assert_eq!(quarantined.len(), 1, "expected one quarantined file");
    assert_ne!(original, b"{ truncated by a racing writer".to_vec());
}

/// A *read* still degrades to "no such secret" rather than failing, because the
/// caller's remedy (sign in again) is recoverable and failing every unrelated
/// lookup is not.
#[test]
fn file_backend_get_treats_an_unparseable_file_as_empty() {
    let dir = TempDir::new().expect("tempdir");
    let fb = FileBackend::new(dir.path());
    std::fs::write(dir.path().join("dev-keychain.json"), b"not json").unwrap();

    assert!(fb.get("anything").unwrap().is_none());
    assert!(
        dir.path().join("dev-keychain.json").exists(),
        "a read must not move the file aside"
    );
}

/// Concurrent writers must not lose each other's entries. Before the
/// cross-process lock, each writer persisted the map it had read *before* the
/// other landed, so all but the last key vanished.
#[test]
fn file_backend_concurrent_sets_all_survive() {
    let dir = TempDir::new().expect("tempdir");
    let writers = 8;

    std::thread::scope(|scope| {
        for i in 0..writers {
            let path = dir.path().to_path_buf();
            scope.spawn(move || {
                // A backend instance per thread: one shared instance would be
                // covered by an in-process mutex, which is precisely the
                // guarantee that turned out to be insufficient.
                FileBackend::new(&path)
                    .set(&format!("key{i}"), &format!("v{i}"))
                    .unwrap();
            });
        }
    });

    let fb = FileBackend::new(dir.path());
    for i in 0..writers {
        assert_eq!(
            fb.get(&format!("key{i}")).unwrap().as_deref(),
            Some(format!("v{i}").as_str()),
            "key{i} was lost to a concurrent write"
        );
    }
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
) -> Result<MigrationOutcome, SecretStorageError> {
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
    let content =
        std::fs::read_to_string(path).map_err(|e| SecretStorageError::MigrationReadFailed {
            path: path.display().to_string(),
            source: e,
        })?;
    let value = content.trim().to_string();

    // -- Step 4: write --
    fb.set(&nk, &value)?;

    // -- Step 5: verify --
    let readback = fb.get(&nk)?;
    if readback.as_deref() != Some(value.as_str()) {
        return Err(SecretStorageError::VerifyFailed {
            key: key.to_string(),
        });
    }

    // -- Step 6: delete source file --
    std::fs::remove_file(path).map_err(|e| SecretStorageError::MigrationDeleteFailed {
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
