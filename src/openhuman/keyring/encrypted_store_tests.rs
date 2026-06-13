use super::*;
use tempfile::TempDir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

// ── SecretStore basics ─────────────────────────────────────

#[test]
fn encrypt_decrypt_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);
    let secret = "sk-my-secret-api-key-12345";

    let encrypted = store.encrypt(secret).unwrap();
    assert!(encrypted.starts_with("enc2:"), "Should have enc2: prefix");
    assert_ne!(encrypted, secret, "Should not be plaintext");

    let decrypted = store.decrypt(&encrypted).unwrap();
    assert_eq!(decrypted, secret, "Roundtrip must preserve original");
}

#[test]
fn encrypt_empty_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);
    let result = store.encrypt("").unwrap();
    assert_eq!(result, "");
}

#[test]
fn decrypt_plaintext_passthrough() {
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);
    // Values without "enc:"/"enc2:" prefix are returned as-is (backward compat)
    let result = store.decrypt("sk-plaintext-key").unwrap();
    assert_eq!(result, "sk-plaintext-key");
}

#[test]
fn disabled_store_returns_plaintext() {
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), false);
    let result = store.encrypt("sk-secret").unwrap();
    assert_eq!(result, "sk-secret", "Disabled store should not encrypt");
}

#[test]
fn keyring_user_id_uses_last_path_component_when_available() {
    let path = std::path::Path::new("/tmp/openhuman/users/user-123");
    assert_eq!(keyring_user_id_from_dir(path), "user-123");
}

#[test]
fn keyring_user_id_falls_back_to_stable_hash() {
    let path = std::path::Path::new("/");
    let first = keyring_user_id_from_dir(path);
    let second = keyring_user_id_from_dir(path);
    assert_eq!(first, second);
    assert!(first.starts_with("secretstore-path-"));
}

#[test]
fn is_encrypted_detects_prefix() {
    assert!(SecretStore::is_encrypted("enc2:aabbcc"));
    assert!(SecretStore::is_encrypted("enc:aabbcc")); // legacy
    assert!(!SecretStore::is_encrypted("sk-plaintext"));
    assert!(!SecretStore::is_encrypted(""));
}

#[tokio::test]
async fn key_file_created_on_first_encrypt() {
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);
    assert!(!store.key_path.exists());

    store.encrypt("test").unwrap();
    assert!(store.key_path.exists(), "Key file should be created");

    let key_hex = tokio::fs::read_to_string(&store.key_path).await.unwrap();
    assert_eq!(
        key_hex.len(),
        KEY_LEN * 2,
        "Key should be {KEY_LEN} bytes hex-encoded"
    );
}

#[cfg(unix)]
#[test]
fn key_file_is_created_with_owner_only_permissions() {
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);

    store.encrypt("test").unwrap();

    let metadata = std::fs::metadata(&store.key_path).unwrap();
    assert_eq!(
        metadata.permissions().mode() & 0o777,
        0o600,
        "Key file must be owner-readable and owner-writable only"
    );
}

#[test]
fn encrypting_same_value_produces_different_ciphertext() {
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);

    let e1 = store.encrypt("secret").unwrap();
    let e2 = store.encrypt("secret").unwrap();
    assert_ne!(
        e1, e2,
        "AEAD with random nonce should produce different ciphertext each time"
    );

    // Both should still decrypt to the same value
    assert_eq!(store.decrypt(&e1).unwrap(), "secret");
    assert_eq!(store.decrypt(&e2).unwrap(), "secret");
}

#[test]
fn different_stores_same_dir_interop() {
    let tmp = TempDir::new().unwrap();
    let store1 = SecretStore::new(tmp.path(), true);
    let store2 = SecretStore::new(tmp.path(), true);

    let encrypted = store1.encrypt("cross-store-secret").unwrap();
    let decrypted = store2.decrypt(&encrypted).unwrap();
    assert_eq!(decrypted, "cross-store-secret");
}

#[test]
fn unicode_secret_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);
    let secret = "sk-日本語テスト-émojis-🦀";

    let encrypted = store.encrypt(secret).unwrap();
    let decrypted = store.decrypt(&encrypted).unwrap();
    assert_eq!(decrypted, secret);
}

#[test]
fn long_secret_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);
    let secret = "a".repeat(10_000);

    let encrypted = store.encrypt(&secret).unwrap();
    let decrypted = store.decrypt(&encrypted).unwrap();
    assert_eq!(decrypted, secret);
}

#[test]
fn corrupt_hex_returns_error() {
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);
    let result = store.decrypt("enc2:not-valid-hex!!");
    assert!(result.is_err());
}

#[test]
fn tampered_ciphertext_detected() {
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);
    let encrypted = store.encrypt("sensitive-data").unwrap();

    // Flip a bit in the ciphertext (after the "enc2:" prefix)
    let hex_str = &encrypted[5..];
    let mut blob = hex_decode(hex_str).unwrap();
    // Modify a byte in the ciphertext portion (after the 12-byte nonce)
    if blob.len() > NONCE_LEN {
        blob[NONCE_LEN] ^= 0xff;
    }
    let tampered = format!("enc2:{}", hex_encode(&blob));

    let result = store.decrypt(&tampered);
    assert!(result.is_err(), "Tampered ciphertext must be rejected");
}

#[test]
fn wrong_key_detected() {
    let tmp1 = TempDir::new().unwrap();
    let tmp2 = TempDir::new().unwrap();
    let store1 = SecretStore::new(tmp1.path(), true);
    let store2 = SecretStore::new(tmp2.path(), true);

    let encrypted = store1.encrypt("secret-for-store1").unwrap();
    let result = store2.decrypt(&encrypted);
    assert!(result.is_err(), "Decrypting with a different key must fail");
}

#[test]
fn truncated_ciphertext_returns_error() {
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);
    // Only a few bytes — shorter than nonce
    let result = store.decrypt("enc2:aabbccdd");
    assert!(result.is_err(), "Too-short ciphertext must be rejected");
}

// ── Legacy XOR backward compatibility ───────────────────────

#[test]
fn legacy_xor_decrypt_still_works() {
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);

    // Trigger key creation via an encrypt call
    let _ = store.encrypt("setup").unwrap();
    let key = store.load_or_create_key().unwrap();

    // Manually produce a legacy XOR-encrypted value
    let plaintext = "sk-legacy-api-key";
    let ciphertext = xor_cipher(plaintext.as_bytes(), &key);
    let legacy_value = format!("enc:{}", hex_encode(&ciphertext));

    // Store should still be able to decrypt legacy values
    let decrypted = store.decrypt(&legacy_value).unwrap();
    assert_eq!(decrypted, plaintext, "Legacy XOR values must still decrypt");
}

// ── Migration tests ─────────────────────────────────────────

#[test]
fn needs_migration_detects_legacy_prefix() {
    assert!(SecretStore::needs_migration("enc:aabbcc"));
    assert!(!SecretStore::needs_migration("enc2:aabbcc"));
    assert!(!SecretStore::needs_migration("sk-plaintext"));
    assert!(!SecretStore::needs_migration(""));
}

#[test]
fn is_secure_encrypted_detects_enc2_only() {
    assert!(SecretStore::is_secure_encrypted("enc2:aabbcc"));
    assert!(!SecretStore::is_secure_encrypted("enc:aabbcc"));
    assert!(!SecretStore::is_secure_encrypted("sk-plaintext"));
    assert!(!SecretStore::is_secure_encrypted(""));
}

#[test]
fn decrypt_and_migrate_returns_none_for_enc2() {
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);

    let encrypted = store.encrypt("my-secret").unwrap();
    assert!(encrypted.starts_with("enc2:"));

    let (plaintext, migrated) = store.decrypt_and_migrate(&encrypted).unwrap();
    assert_eq!(plaintext, "my-secret");
    assert!(
        migrated.is_none(),
        "enc2: values should not trigger migration"
    );
}

#[test]
fn decrypt_and_migrate_returns_none_for_plaintext() {
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);

    let (plaintext, migrated) = store.decrypt_and_migrate("sk-plaintext-key").unwrap();
    assert_eq!(plaintext, "sk-plaintext-key");
    assert!(
        migrated.is_none(),
        "Plaintext values should not trigger migration"
    );
}

#[test]
fn decrypt_and_migrate_upgrades_legacy_xor() {
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);

    // Create key first
    let _ = store.encrypt("setup").unwrap();
    let key = store.load_or_create_key().unwrap();

    // Manually create a legacy XOR-encrypted value
    let plaintext = "sk-legacy-secret-to-migrate";
    let ciphertext = xor_cipher(plaintext.as_bytes(), &key);
    let legacy_value = format!("enc:{}", hex_encode(&ciphertext));

    // Verify it needs migration
    assert!(SecretStore::needs_migration(&legacy_value));

    // Decrypt and migrate
    let (decrypted, migrated) = store.decrypt_and_migrate(&legacy_value).unwrap();
    assert_eq!(decrypted, plaintext, "Plaintext must match original");
    assert!(migrated.is_some(), "Legacy value should trigger migration");

    let new_value = migrated.unwrap();
    assert!(
        new_value.starts_with("enc2:"),
        "Migrated value must use enc2: prefix"
    );
    assert!(
        !SecretStore::needs_migration(&new_value),
        "Migrated value should not need migration"
    );

    // Verify the migrated value decrypts correctly
    let (decrypted2, migrated2) = store.decrypt_and_migrate(&new_value).unwrap();
    assert_eq!(
        decrypted2, plaintext,
        "Migrated value must decrypt to same plaintext"
    );
    assert!(
        migrated2.is_none(),
        "Migrated value should not trigger another migration"
    );
}

#[test]
fn decrypt_and_migrate_handles_unicode() {
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);

    let _ = store.encrypt("setup").unwrap();
    let key = store.load_or_create_key().unwrap();

    let plaintext = "sk-日本語-émojis-🦀-тест";
    let ciphertext = xor_cipher(plaintext.as_bytes(), &key);
    let legacy_value = format!("enc:{}", hex_encode(&ciphertext));

    let (decrypted, migrated) = store.decrypt_and_migrate(&legacy_value).unwrap();
    assert_eq!(decrypted, plaintext);
    assert!(migrated.is_some());

    // Verify migrated value works
    let new_value = migrated.unwrap();
    let (decrypted2, _) = store.decrypt_and_migrate(&new_value).unwrap();
    assert_eq!(decrypted2, plaintext);
}

#[test]
fn decrypt_and_migrate_handles_empty_secret() {
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);

    let _ = store.encrypt("setup").unwrap();
    let key = store.load_or_create_key().unwrap();

    // Empty plaintext XOR-encrypted
    let plaintext = "";
    let ciphertext = xor_cipher(plaintext.as_bytes(), &key);
    let legacy_value = format!("enc:{}", hex_encode(&ciphertext));

    let (decrypted, migrated) = store.decrypt_and_migrate(&legacy_value).unwrap();
    assert_eq!(decrypted, plaintext);
    // Empty string encryption returns empty string (not enc2:)
    assert!(migrated.is_some());
    assert_eq!(migrated.unwrap(), "");
}

#[test]
fn decrypt_and_migrate_handles_long_secret() {
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);

    let _ = store.encrypt("setup").unwrap();
    let key = store.load_or_create_key().unwrap();

    let plaintext = "a".repeat(10_000);
    let ciphertext = xor_cipher(plaintext.as_bytes(), &key);
    let legacy_value = format!("enc:{}", hex_encode(&ciphertext));

    let (decrypted, migrated) = store.decrypt_and_migrate(&legacy_value).unwrap();
    assert_eq!(decrypted, plaintext);
    assert!(migrated.is_some());

    let new_value = migrated.unwrap();
    let (decrypted2, _) = store.decrypt_and_migrate(&new_value).unwrap();
    assert_eq!(decrypted2, plaintext);
}

#[test]
fn decrypt_and_migrate_fails_on_corrupt_legacy_hex() {
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);
    let _ = store.encrypt("setup").unwrap();

    let result = store.decrypt_and_migrate("enc:not-valid-hex!!");
    assert!(result.is_err(), "Corrupt hex should fail");
}

#[test]
fn decrypt_and_migrate_wrong_key_produces_garbage_or_fails() {
    let tmp1 = TempDir::new().unwrap();
    let tmp2 = TempDir::new().unwrap();
    let store1 = SecretStore::new(tmp1.path(), true);
    let store2 = SecretStore::new(tmp2.path(), true);

    // Create keys for both stores
    let _ = store1.encrypt("setup").unwrap();
    let _ = store2.encrypt("setup").unwrap();
    let key1 = store1.load_or_create_key().unwrap();

    // Encrypt with store1's key
    let plaintext = "secret-for-store1";
    let ciphertext = xor_cipher(plaintext.as_bytes(), &key1);
    let legacy_value = format!("enc:{}", hex_encode(&ciphertext));

    // Decrypt with store2 — XOR will produce garbage bytes
    // This may fail with UTF-8 error or succeed with garbage plaintext
    match store2.decrypt_and_migrate(&legacy_value) {
        Ok((decrypted, _)) => {
            // If it succeeds, the plaintext should be garbage (not the original)
            assert_ne!(
                decrypted, plaintext,
                "Wrong key should produce garbage plaintext"
            );
        }
        Err(e) => {
            // Expected: UTF-8 decoding failure from garbage bytes
            assert!(
                e.to_string().contains("UTF-8"),
                "Error should be UTF-8 related: {e}"
            );
        }
    }
}

#[test]
fn migration_produces_different_ciphertext_each_time() {
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);

    let _ = store.encrypt("setup").unwrap();
    let key = store.load_or_create_key().unwrap();

    let plaintext = "sk-same-secret";
    let ciphertext = xor_cipher(plaintext.as_bytes(), &key);
    let legacy_value = format!("enc:{}", hex_encode(&ciphertext));

    let (_, migrated1) = store.decrypt_and_migrate(&legacy_value).unwrap();
    let (_, migrated2) = store.decrypt_and_migrate(&legacy_value).unwrap();

    assert!(migrated1.is_some());
    assert!(migrated2.is_some());
    assert_ne!(
        migrated1.unwrap(),
        migrated2.unwrap(),
        "Each migration should produce different ciphertext (random nonce)"
    );
}

#[test]
fn migrated_value_is_tamper_resistant() {
    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);

    let _ = store.encrypt("setup").unwrap();
    let key = store.load_or_create_key().unwrap();

    let plaintext = "sk-sensitive-data";
    let ciphertext = xor_cipher(plaintext.as_bytes(), &key);
    let legacy_value = format!("enc:{}", hex_encode(&ciphertext));

    let (_, migrated) = store.decrypt_and_migrate(&legacy_value).unwrap();
    let new_value = migrated.unwrap();

    // Tamper with the migrated value
    let hex_str = &new_value[5..];
    let mut blob = hex_decode(hex_str).unwrap();
    if blob.len() > NONCE_LEN {
        blob[NONCE_LEN] ^= 0xff;
    }
    let tampered = format!("enc2:{}", hex_encode(&blob));

    let result = store.decrypt_and_migrate(&tampered);
    assert!(result.is_err(), "Tampered migrated value must be rejected");
}

// ── Low-level helpers ───────────────────────────────────────

#[test]
fn xor_cipher_roundtrip() {
    let key = b"testkey123";
    let data = b"hello world";
    let encrypted = xor_cipher(data, key);
    let decrypted = xor_cipher(&encrypted, key);
    assert_eq!(decrypted, data);
}

#[test]
fn xor_cipher_empty_key() {
    let data = b"passthrough";
    let result = xor_cipher(data, &[]);
    assert_eq!(result, data);
}

#[test]
fn hex_roundtrip() {
    let data = vec![0x00, 0x01, 0xfe, 0xff, 0xab, 0xcd];
    let encoded = hex_encode(&data);
    assert_eq!(encoded, "0001feffabcd");
    let decoded = hex_decode(&encoded).unwrap();
    assert_eq!(decoded, data);
}

#[test]
fn hex_decode_odd_length_fails() {
    assert!(hex_decode("abc").is_err());
}

#[test]
fn hex_decode_invalid_chars_fails() {
    assert!(hex_decode("zzzz").is_err());
}

#[test]
fn windows_icacls_grant_arg_rejects_empty_username() {
    assert_eq!(build_windows_icacls_grant_arg(""), None);
    assert_eq!(build_windows_icacls_grant_arg("   \t\n"), None);
}

#[test]
fn windows_icacls_grant_arg_trims_username() {
    assert_eq!(
        build_windows_icacls_grant_arg("  alice  "),
        Some("alice:F".to_string())
    );
}

#[test]
fn windows_icacls_grant_arg_preserves_valid_characters() {
    assert_eq!(
        build_windows_icacls_grant_arg("DOMAIN\\svc-user"),
        Some("DOMAIN\\svc-user:F".to_string())
    );
}

// ── qualify_windows_username ─────────────────────────────────

#[cfg(windows)]
#[test]
fn qualify_windows_username_local_account() {
    // USERDOMAIN == COMPUTERNAME → standalone machine → plain username
    assert_eq!(
        qualify_windows_username("alice", "DESKTOP-ABC", "DESKTOP-ABC"),
        "alice"
    );
}

#[cfg(windows)]
#[test]
fn qualify_windows_username_domain_joined() {
    // USERDOMAIN != COMPUTERNAME → domain-joined → prefix with domain
    assert_eq!(
        qualify_windows_username("alice", "CORP", "DESKTOP-ABC"),
        "CORP\\alice"
    );
}

#[cfg(windows)]
#[test]
fn qualify_windows_username_case_insensitive_comparison() {
    // Case-insensitive: "desktop-abc" == "DESKTOP-ABC" → local account
    assert_eq!(
        qualify_windows_username("bob", "desktop-abc", "DESKTOP-ABC"),
        "bob"
    );
}

#[cfg(windows)]
#[test]
fn qualify_windows_username_empty_computername() {
    // COMPUTERNAME is unset — fall back to plain username to avoid prefixing
    // with a potentially meaningless domain string
    assert_eq!(qualify_windows_username("alice", "CORP", ""), "alice");
}

#[cfg(windows)]
#[test]
fn qualify_windows_username_empty_userdomain() {
    // USERDOMAIN is unset — use plain username
    assert_eq!(
        qualify_windows_username("alice", "", "DESKTOP-ABC"),
        "alice"
    );
}

#[cfg(windows)]
#[test]
fn qualify_windows_username_empty_username_returns_empty() {
    assert_eq!(qualify_windows_username("", "CORP", "DESKTOP-ABC"), "");
}

#[cfg(windows)]
#[test]
fn qualify_windows_username_whitespace_trimmed() {
    assert_eq!(
        qualify_windows_username("  alice  ", "  CORP  ", "  DESKTOP-XYZ  "),
        "CORP\\alice"
    );
}

// ── Windows self-repair path ─────────────────────────────────

/// Simulate a locked key file on non-Windows: write the file, remove all
/// read permissions, verify the store recovers after `chmod` restores them.
/// On Windows the equivalent is tested by is_permission_error / repair_windows_acl.
#[cfg(unix)]
#[test]
fn locked_key_file_fails_gracefully_on_unix() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().unwrap();
    let store = SecretStore::new(tmp.path(), true);

    // Trigger key creation so the file exists on disk.
    let encrypted = store.encrypt("original-secret").unwrap();
    assert!(store.key_path.exists());

    // Lock the file before clearing the cache, so the next decrypt must read
    // from disk and encounter the PermissionDenied error.
    fs::set_permissions(&store.key_path, fs::Permissions::from_mode(0o000)).unwrap();

    // Clear the cache so the decrypt path actually hits the disk.
    super::clear_cached_key(&store.key_path);

    // Linux CI containers commonly run as root, which bypasses file permission
    // checks — chmod 0o000 has no effect and the file stays readable.  Only
    // assert the graceful-failure behaviour when the lock actually took hold;
    // otherwise the test would fail vacuously on root runners.
    let file_is_locked = fs::read_to_string(&store.key_path).is_err();
    if file_is_locked {
        let result = store.decrypt(&encrypted);
        assert!(
            result.is_err(),
            "decrypt must fail gracefully when key file is locked and cache is empty"
        );
    }

    // Restore permissions so TempDir cleanup can remove the file.
    fs::set_permissions(&store.key_path, fs::Permissions::from_mode(0o600)).unwrap();
}

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
    super::clear_cached_key(&store.key_path);

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
        super::clear_cached_key(&store.key_path);
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
        let repaired = super::repair_windows_acl(&store.key_path);
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
    super::clear_cached_key(&store.key_path);

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
    super::clear_cached_key(&store.key_path);
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
    super::clear_cached_key(&store.key_path);

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
