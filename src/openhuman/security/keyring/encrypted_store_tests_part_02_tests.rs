use super::*;

/// End-to-end test for the Windows self-repair path.
///
/// Recreates the exact bad state that caused OPENHUMAN-TAURI-GN:
///   1. Key file created, ACL corrupted with `icacls /inheritance:r` + no valid grant
///      (simulated here with an explicit `Everyone:DENY` which is even stricter).
///   2. In-memory cache cleared so the next call must actually read from disk.
///   3. `decrypt` is called — the self-repair path must run `icacls /reset`,
///      restore inherited ACLs, re-read the file, and return the correct plaintext.
///
/// The lock step may be a no-op when the test process runs as SYSTEM/Administrator
/// (elevated tokens bypass DENY ACEs).  In that case the test skips the
/// "verify locked" assertion and still validates that repair_windows_acl + decrypt
/// complete without panicking or returning an unexpected error.
///
/// Run on Windows CI via the `rust-core-tests-windows` job in test-reusable.yml.
#[cfg(windows)]
#[test]
fn self_repair_recovers_from_locked_key_file() {
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);

    // Step 1: create the key file and produce a ciphertext to decrypt later.
    let encrypted = store
        .encrypt("secret-to-survive-acl-lockout")
        .expect("initial encrypt must succeed");
    assert!(
        store.key_path.exists(),
        "key file must exist after first encrypt"
    );

    // Step 2: clear the in-memory cache so the next decrypt reads from disk.
    super::super::clear_cached_key(&store.key_path);

    // Step 3: corrupt the ACL — strip inheritance AND add an explicit DENY for
    // Everyone.  This is a strict superset of the production failure mode (where
    // /inheritance:r ran but the /grant target was unresolvable, leaving no ACE).
    let lock_status = std::process::Command::new("icacls")
        .arg(&store.key_path)
        .args(["/inheritance:r", "/deny"])
        .arg("Everyone:F")
        .status()
        .expect("icacls must be available on Windows");
    assert!(
        lock_status.success(),
        "icacls lock step must succeed — test setup invalid"
    );

    // Step 4: check whether the lock actually made the file unreadable.
    // Elevated (SYSTEM/admin) tokens bypass DENY ACEs, so on those runners
    // the file stays readable and we skip the self-repair assertion — but we
    // still validate repair_windows_acl completes cleanly (no panic).
    let file_is_locked = fs::read_to_string(&store.key_path).is_err();

    if file_is_locked {
        // Full E2E path: self-repair must restore access and return plaintext.
        let decrypted = store
            .decrypt(&encrypted)
            .expect("self-repair must restore access and return correct plaintext");
        assert_eq!(
            decrypted, "secret-to-survive-acl-lockout",
            "decrypted value must match original"
        );
        // Verify the repair is durable: clear the in-memory cache and decrypt a
        // second time from disk.  If the ACL is truly fixed, this succeeds on the
        // first read attempt without triggering the repair path again.  (A direct
        // fs::read_to_string assertion here is flaky — Windows Defender / the
        // Security Center can briefly re-acquire the file handle right after an
        // icacls operation, causing intermittent PermissionDenied.  Going through
        // load_or_create_key means the retry backoff in read_key_file_with_retry
        // absorbs that transient window, which is exactly what production code does.)
        super::super::clear_cached_key(&store.key_path);
        let decrypted2 = store
            .decrypt(&encrypted)
            .expect("ACL fix must be durable: second from-disk decrypt must succeed");
        assert_eq!(
            decrypted2, "secret-to-survive-acl-lockout",
            "second decrypt must return the same plaintext"
        );
    } else {
        // Elevated runner: lock was bypassed.  Verify repair_windows_acl runs
        // cleanly on an already-accessible file (icacls /reset is idempotent).
        let repaired = super::super::repair_windows_acl(&store.key_path);
        assert!(
            repaired,
            "repair_windows_acl must succeed on an accessible file"
        );
        let decrypted = store
            .decrypt(&encrypted)
            .expect("decrypt must succeed when file is accessible");
        assert_eq!(decrypted, "secret-to-survive-acl-lockout");
    }
}

/// Verify that the self-repair path does NOT trigger for non-permission errors
/// (e.g. corrupt/truncated file) — we should get a clear error, not a silent
/// retry that produces garbage.
#[cfg(windows)]
#[test]
fn self_repair_does_not_trigger_for_corrupt_file() {
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);

    // Write a corrupt (non-hex) key file directly — simulates on-disk corruption.
    fs::create_dir_all(tmp.path()).unwrap();
    fs::write(&store.key_path, "this-is-not-valid-hex!!!").unwrap();
    super::super::clear_cached_key(&store.key_path);

    let err = store.encrypt("anything").unwrap_err();
    let msg = format!("{err:?}");
    // Must surface a hex/corrupt error, not attempt a repair loop.
    assert!(
        msg.contains("corrupt") || msg.contains("hex") || msg.contains("Invalid"),
        "corrupt file must surface a clear decode error, got: {msg}"
    );
}

#[cfg(windows)]
#[test]
fn is_permission_error_matches_access_denied() {
    use std::io::{Error, ErrorKind};
    let perm_err = Error::from(ErrorKind::PermissionDenied);
    assert!(is_permission_error(&perm_err));
}

#[cfg(windows)]
#[test]
fn is_permission_error_ignores_not_found() {
    use std::io::{Error, ErrorKind};
    let not_found = Error::from(ErrorKind::NotFound);
    assert!(!is_permission_error(&not_found));
}

#[cfg(windows)]
#[test]
fn is_permission_error_matches_raw_os_error_5() {
    use std::io::Error;
    // raw OS error 5 = ERROR_ACCESS_DENIED
    let err = Error::from_raw_os_error(5);
    assert!(is_permission_error(&err));
}

#[test]
fn generate_random_key_correct_length() {
    let key = generate_random_key();
    assert_eq!(key.len(), KEY_LEN);
}

#[test]
fn master_key_is_zeroizing_and_cache_stable() {
    // Regression for audit C9: the master key is returned wrapped in
    // `zeroize::Zeroizing` (wiped on drop) and the cached copy is the same
    // bytes on a subsequent load (cache hit). The static type assertion below
    // fails to compile if the return type stops being `Zeroizing`.
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);

    let key: zeroize::Zeroizing<Vec<u8>> = store.load_or_create_key().unwrap();
    assert_eq!(key.len(), KEY_LEN);

    // Second load must return identical key bytes from the process-wide cache.
    let key2 = store.load_or_create_key().unwrap();
    assert_eq!(
        &*key, &*key2,
        "cached master key must be stable across loads"
    );
}

#[test]
fn generate_random_key_not_all_zeros() {
    let key = generate_random_key();
    assert!(key.iter().any(|&b| b != 0), "Key should not be all zeros");
}

#[test]
fn two_random_keys_differ() {
    let k1 = generate_random_key();
    let k2 = generate_random_key();
    assert_ne!(k1, k2, "Two random keys should differ");
}

#[test]
fn generate_random_key_has_no_uuid_fixed_bits() {
    // UUID v4 has fixed bits at positions 6 (version = 0b0100xxxx) and
    // 8 (variant = 0b10xxxxxx). A direct CSPRNG key should not consistently
    // have these patterns across multiple samples.
    let mut version_match = 0;
    let mut variant_match = 0;
    let samples = 100;
    for _ in 0..samples {
        let key = generate_random_key();
        // In UUID v4, byte 6 always has top nibble = 0x4
        if key[6] & 0xf0 == 0x40 {
            version_match += 1;
        }
        // In UUID v4, byte 8 always has top 2 bits = 0b10
        if key[8] & 0xc0 == 0x80 {
            variant_match += 1;
        }
    }
    // With true randomness, each pattern should appear ~1/16 and ~1/4 of
    // the time. UUID would hit 100/100 on both. Allow generous margin.
    assert!(
        version_match < 30,
        "byte[6] matched UUID v4 version nibble {version_match}/100 times — \
         likely still using UUID-based key generation"
    );
    assert!(
        variant_match < 50,
        "byte[8] matched UUID v4 variant bits {variant_match}/100 times — \
         likely still using UUID-based key generation"
    );
}

#[test]
fn key_loaded_once_then_cached() {
    // After the first read, subsequent decrypts must not depend on the key
    // file being readable. This is the property that protects us from
    // transient Windows sharing violations on `.secret_key` (Sentry
    // OPENHUMAN-TAURI-58: "Failed to read secret key file" hammering
    // app_state_snapshot).
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);

    let encrypted = store.encrypt("cached-secret").unwrap();
    assert!(store.key_path.exists());

    // Make the file unreadable by deleting it — the in-memory cache should
    // still satisfy the decrypt.
    fs::remove_file(&store.key_path).unwrap();
    let decrypted = store.decrypt(&encrypted).unwrap();
    assert_eq!(decrypted, "cached-secret");

    // After clearing the cache, the disappearance is visible again: the
    // store falls back to the "create new key" branch and decryption with
    // the original ciphertext fails.
    super::super::clear_cached_key(&store.key_path);
    let result = store.decrypt(&encrypted);
    assert!(
        result.is_err(),
        "Without cache and without file, decrypt must fail"
    );
}

#[test]
fn malformed_key_file_rejected_not_panic() {
    // hex_decode only checks the string is even-length, so a truncated /
    // padded key file would previously sail through and panic later inside
    // `Key::from_slice` (ChaCha20-Poly1305 requires exactly 32 bytes).
    // Verify we now reject with a clean error.
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);

    // Write a 30-byte hex key (60 chars, even, decodes cleanly, wrong length).
    fs::create_dir_all(&tmp.path()).unwrap();
    fs::write(&store.key_path, "aa".repeat(30)).unwrap();
    super::super::clear_cached_key(&store.key_path);

    let err = store.encrypt("anything").unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("wrong length"),
        "expected wrong-length error, got: {msg}"
    );
}

#[cfg(unix)]
#[test]
fn key_file_has_restricted_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);
    store.encrypt("trigger key creation").unwrap();

    let perms = fs::metadata(&store.key_path).unwrap().permissions();
    assert_eq!(
        perms.mode() & 0o777,
        0o600,
        "Key file must be owner-only (0600)"
    );
}
