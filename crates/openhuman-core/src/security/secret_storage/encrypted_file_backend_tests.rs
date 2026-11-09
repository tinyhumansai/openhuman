use super::*;
use crate::security::secret_storage::file_store;
use std::collections::HashMap;

fn secrets_tempdir() -> tempfile::TempDir {
    let dir = {
        #[cfg(target_os = "macos")]
        {
            tempfile::tempdir_in("/private/tmp").expect("canonical macOS temp directory")
        }
        #[cfg(not(target_os = "macos"))]
        {
            tempfile::tempdir().expect("tempdir")
        }
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .expect("make secret fixture directory private");
    }
    dir
}

fn write_private_file(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}

fn legacy_secret_map() -> HashMap<String, String> {
    HashMap::from([("user:token".to_string(), "secret-value".to_string())])
}

fn read_secret(backend: &EncryptedFileBackend, key: &[u8; KEY_LEN]) -> Option<String> {
    backend.get_with_key(key, "user:token").unwrap()
}

fn write_secret(backend: &EncryptedFileBackend, key: &[u8; KEY_LEN], value: &str) {
    backend.set_with_key(key, "user:token", value).unwrap();
}

fn write_profile_key(workspace: &Path, key: &[u8; KEY_LEN]) {
    let path = workspace.join("keyring/master.key");
    let encoded = crypto::key_to_hex(key);
    file_store::write_atomic(&path, encoded.as_bytes()).unwrap();
}

struct DeniedMasterKeySource;

impl key_source::MasterKeySource for DeniedMasterKeySource {
    fn load(&self) -> Result<Option<Zeroizing<[u8; KEY_LEN]>>, SecretStorageError> {
        Err(SecretStorageError::Backend("access denied".into()))
    }

    fn store(&self, _: &[u8; KEY_LEN]) -> Result<(), SecretStorageError> {
        panic!("a denied source must not be written")
    }
}

struct ClearMasterKeySourceOnDrop;

impl Drop for ClearMasterKeySourceOnDrop {
    fn drop(&mut self) {
        key_source::clear_master_key_source();
    }
}

#[test]
fn encrypted_get_reports_unavailable_master_key() {
    let workspace = secrets_tempdir();
    let backend = EncryptedFileBackend::new(workspace.path());
    let key = [0x42; KEY_LEN];
    write_secret(&backend, &key, "still-present");

    key_source::install_master_key_source(std::sync::Arc::new(DeniedMasterKeySource));
    let _clear = ClearMasterKeySourceOnDrop;

    assert!(matches!(
        backend.get("user:token"),
        Err(SecretStorageError::MasterKeyUnavailable)
    ));
}

#[test]
fn encrypted_delete_reports_unavailable_master_key_and_preserves_file() {
    let workspace = secrets_tempdir();
    let backend = EncryptedFileBackend::new(workspace.path());
    let key = [0x42; KEY_LEN];
    write_secret(&backend, &key, "still-present");
    let original = std::fs::read(&backend.path).unwrap();

    key_source::install_master_key_source(std::sync::Arc::new(DeniedMasterKeySource));
    let _clear = ClearMasterKeySourceOnDrop;

    assert!(matches!(
        backend.delete("user:token"),
        Err(SecretStorageError::MasterKeyUnavailable)
    ));
    assert_eq!(std::fs::read(&backend.path).unwrap(), original);
}

#[test]
fn profile_read_only_secret_lookup_uses_only_the_selected_store() {
    let _env = crate::config::test_env::EnvVarGuard::locked_unset_many(&[
        MASTER_KEY_ENV,
        MASTER_KEY_FILE_ENV,
    ]);
    let account_a = secrets_tempdir();
    let account_b = secrets_tempdir();
    let key_a = [0x42; KEY_LEN];
    let key_b = [0x24; KEY_LEN];
    write_profile_key(account_a.path(), &key_a);
    write_profile_key(account_b.path(), &key_b);

    let store_a = EncryptedFileBackend::new(account_a.path());
    let store_b = EncryptedFileBackend::new(account_b.path());
    store_a
        .set_with_key(&key_a, "account:session", "account-a-session")
        .unwrap();
    store_b
        .set_with_key(&key_b, "account:other", "account-b-other")
        .unwrap();
    let legacy = account_b.path().join(LEGACY_DEV_KEYCHAIN);
    std::fs::write(
        &legacy,
        serde_json::to_vec(&HashMap::from([(
            "account:session".to_string(),
            "ambient-session".to_string(),
        )]))
        .unwrap(),
    )
    .unwrap();

    assert_eq!(
        store_b.get_profile_read_only("account:session").unwrap(),
        None
    );
    assert_eq!(
        store_a
            .get_profile_read_only("account:session")
            .unwrap()
            .as_deref(),
        Some("account-a-session")
    );
    assert!(
        legacy.exists(),
        "read-only lookup must not import legacy data"
    );
}

#[test]
fn profile_read_only_secret_lookup_does_not_create_a_missing_key() {
    let _env = crate::config::test_env::EnvVarGuard::locked_unset_many(&[
        MASTER_KEY_ENV,
        MASTER_KEY_FILE_ENV,
    ]);
    let workspace = secrets_tempdir();
    let backend = EncryptedFileBackend::new(workspace.path());

    let error = backend
        .get_profile_read_only("account:session")
        .unwrap_err();

    assert!(error.to_string().contains("master key unavailable"));
    assert!(!workspace.path().join("keyring/master.key").exists());
}

#[test]
fn profile_read_only_secret_lookup_preserves_corrupt_store_bytes() {
    let workspace = secrets_tempdir();
    let key = [0x42; KEY_LEN];
    write_profile_key(workspace.path(), &key);
    let backend = EncryptedFileBackend::new(workspace.path());
    std::fs::write(&backend.path, b"corrupt encrypted store").unwrap();

    assert!(backend.get_profile_read_only("user:token").is_err());

    assert_eq!(
        std::fs::read(&backend.path).unwrap(),
        b"corrupt encrypted store"
    );
    assert!(!std::fs::read_dir(workspace.path())
        .unwrap()
        .flatten()
        .any(|entry| entry
            .file_name()
            .to_string_lossy()
            .starts_with("secrets.enc.corrupt.")));
}

#[test]
fn desktop_secret_store_quarantines_corruption_through_the_driver_policy() {
    let workspace = secrets_tempdir();
    let backend = EncryptedFileBackend::new(workspace.path());
    let path = workspace.path().join("secrets.enc");
    let corrupt = b"not an encrypted secret file";
    write_private_file(&path, corrupt);

    assert_eq!(
        backend
            .get_with_key(&[0x42; KEY_LEN], "user:token")
            .unwrap(),
        None
    );

    assert!(!path.exists());
    let archives: Vec<_> = std::fs::read_dir(workspace.path())
        .unwrap()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("secrets.enc.corrupt.")
        })
        .collect();
    assert_eq!(archives.len(), 1);
    assert_eq!(std::fs::read(archives[0].path()).unwrap(), corrupt);
}

#[test]
fn legacy_plaintext_is_removed_only_after_verified_encrypted_migration() {
    let dir = secrets_tempdir();
    let legacy_path = dir.path().join(LEGACY_DEV_KEYCHAIN);
    let expected = HashMap::from([
        ("user:token".to_string(), "secret-value".to_string()),
        ("user:second".to_string(), "second-value".to_string()),
    ]);
    write_private_file(&legacy_path, &serde_json::to_vec(&expected).unwrap());

    let backend = EncryptedFileBackend::new(dir.path());
    let key = [0x42; KEY_LEN];
    assert_eq!(read_secret(&backend, &key).as_deref(), Some("secret-value"));
    assert_eq!(
        backend
            .get_with_key(&key, "user:second")
            .unwrap()
            .as_deref(),
        Some("second-value")
    );
    assert!(!legacy_path.exists());
    assert!(backend.path.exists());
    assert!(!legacy_path.with_extension("json.migrated").exists());
    assert_eq!(
        backend.get_with_key(&key, LEGACY_IMPORT_PAYLOAD).unwrap(),
        None
    );
    assert_eq!(
        backend
            .get_with_key(&key, LEGACY_IMPORT_COMPLETE)
            .unwrap()
            .as_deref(),
        Some("1")
    );
    assert_eq!(read_secret(&backend, &key).as_deref(), Some("secret-value"));
    assert_eq!(expected["user:token"], "secret-value");
}

#[test]
fn invalid_legacy_plaintext_is_preserved_for_recovery() {
    let dir = secrets_tempdir();
    let legacy_path = dir.path().join(LEGACY_DEV_KEYCHAIN);
    write_private_file(&legacy_path, b"invalid JSON");

    let backend = EncryptedFileBackend::new(dir.path());
    assert!(backend
        .get_with_key(&[0x42; KEY_LEN], "user:token")
        .is_err());
    assert_eq!(std::fs::read(&legacy_path).unwrap(), b"invalid JSON");
    assert!(!backend.path.exists());
}

#[test]
fn a_partial_legacy_replay_failure_keeps_the_source_and_retries_idempotently() {
    let dir = secrets_tempdir();
    let backend = EncryptedFileBackend::new(dir.path());
    let key = [0x42; KEY_LEN];
    let legacy_path = dir.path().join(LEGACY_DEV_KEYCHAIN);
    let expected = HashMap::from([
        ("a:import".to_string(), "imported".to_string()),
        ("z:conflict".to_string(), "legacy".to_string()),
    ]);
    let source_bytes = serde_json::to_vec(&expected).unwrap();
    write_private_file(&legacy_path, &source_bytes);
    backend
        .set_with_key(&key, "z:conflict", "different")
        .unwrap();

    assert!(backend.import_legacy_dev_keychain(&key).is_err());
    assert_eq!(std::fs::read(&legacy_path).unwrap(), source_bytes);
    assert_eq!(
        backend.get_with_key(&key, "a:import").unwrap().as_deref(),
        Some("imported"),
        "earlier entries may replay before a later conflict"
    );
    assert_eq!(
        backend.get_with_key(&key, "z:conflict").unwrap().as_deref(),
        Some("different")
    );

    assert!(backend.import_legacy_dev_keychain(&key).is_err());
    assert_eq!(std::fs::read(&legacy_path).unwrap(), source_bytes);
    assert_eq!(
        backend.get_with_key(&key, "a:import").unwrap().as_deref(),
        Some("imported"),
        "a retry must accept an already-replayed identical value"
    );
}

#[test]
fn encrypted_store_remains_readable_when_verified_plaintext_cannot_be_deleted() {
    let dir = secrets_tempdir();
    let backend = EncryptedFileBackend::new(dir.path());
    let key = [0x42; KEY_LEN];
    let legacy_path = dir.path().join(LEGACY_DEV_KEYCHAIN);
    write_secret(&backend, &key, "secret-value");
    write_private_file(&legacy_path, b"stale invalid plaintext");

    assert_eq!(read_secret(&backend, &key).as_deref(), Some("secret-value"));
    assert_eq!(
        std::fs::read(&legacy_path).unwrap(),
        b"stale invalid plaintext"
    );
}

#[test]
fn older_plaintext_copy_is_removed_only_when_it_matches() {
    let dir = secrets_tempdir();
    let backend = EncryptedFileBackend::new(dir.path());
    let key = [0x42; KEY_LEN];
    let encrypted = legacy_secret_map();
    write_secret(&backend, &key, "secret-value");

    let old_copy = dir
        .path()
        .join(LEGACY_DEV_KEYCHAIN)
        .with_extension("json.migrated");
    write_private_file(&old_copy, &serde_json::to_vec(&encrypted).unwrap());
    assert_eq!(read_secret(&backend, &key).as_deref(), Some("secret-value"));
    assert!(!old_copy.exists());

    write_private_file(&old_copy, b"invalid JSON");
    assert_eq!(read_secret(&backend, &key).as_deref(), Some("secret-value"));
    assert_eq!(std::fs::read(&old_copy).unwrap(), b"invalid JSON");
}

// ── Operator-supplied master key (#6926) ─────────────────────────────────────

fn hex_key(byte: u8) -> String {
    format!("{byte:02x}").repeat(KEY_LEN)
}

fn file_must_not_be_read(path: &Path) -> Result<String, String> {
    panic!("master key file {} must not be read", path.display())
}

#[test]
fn env_inline_key_is_used_and_names_its_source() {
    let hex = hex_key(0xab);
    let (key, source) = master_key_from_env(Some(&hex), None, file_must_not_be_read)
        .expect("valid inline key")
        .expect("inline key counts as supplied");
    assert_eq!(key, [0xabu8; KEY_LEN]);
    assert_eq!(source, MASTER_KEY_ENV);
}

#[test]
fn env_inline_key_tolerates_surrounding_whitespace_and_uppercase_hex() {
    let hex = format!("  {}\n", hex_key(0xcd).to_uppercase());
    let (key, _) = master_key_from_env(Some(&hex), None, file_must_not_be_read)
        .expect("trimmed key is valid")
        .expect("supplied");
    assert_eq!(key, [0xcdu8; KEY_LEN]);
}

#[test]
fn env_file_key_is_read_trimmed_and_names_the_variable() {
    let hex = hex_key(0x11);
    let (key, source) =
        master_key_from_env(None, Some("/run/secrets/openhuman_master_key"), |path| {
            assert_eq!(path, Path::new("/run/secrets/openhuman_master_key"));
            Ok(format!("{hex}\n"))
        })
        .expect("file key is valid")
        .expect("supplied");
    assert_eq!(key, [0x11u8; KEY_LEN]);
    assert_eq!(source, MASTER_KEY_FILE_ENV);
}

#[test]
fn unreadable_env_file_is_an_error_that_names_the_variable_not_the_path() {
    let err = master_key_from_env(None, Some("/nonexistent/master.key"), |_| {
        Err("cannot read master key file: boom".to_string())
    })
    .expect_err("unreadable file must not fall through to the keychain");
    assert!(err.contains(MASTER_KEY_FILE_ENV), "{err}");
    assert!(err.contains("cannot read"), "{err}");
    assert!(!err.contains("/nonexistent/master.key"), "{err}");
}

#[test]
fn a_key_pasted_into_the_file_variable_is_not_echoed() {
    // The operator meant `MASTER_KEY=<hex>` but set `MASTER_KEY_FILE=<hex>`:
    // the "path" is the secret, so the read error must not carry it into the
    // startup log.
    let pasted_key = hex_key(0x77);
    let err = master_key_from_env(None, Some(&pasted_key), read_master_key_file)
        .expect_err("a key is not a readable path");
    assert!(err.contains(MASTER_KEY_FILE_ENV), "{err}");
    assert!(
        !err.contains(&pasted_key),
        "error must not echo the key: {err}"
    );
}

#[test]
fn both_env_sources_set_is_rejected_before_reading_anything() {
    let hex = hex_key(0x22);
    let err = master_key_from_env(Some(&hex), Some("/run/secrets/key"), file_must_not_be_read)
        .expect_err("ambiguous configuration must be rejected");
    assert!(
        err.contains(MASTER_KEY_ENV) && err.contains(MASTER_KEY_FILE_ENV),
        "{err}"
    );
}

#[test]
fn malformed_env_key_is_rejected_without_leaking_the_value() {
    let too_short = "abcd";
    let err = master_key_from_env(Some(too_short), None, file_must_not_be_read)
        .expect_err("wrong length is rejected");
    assert!(err.contains("expected 64 hex characters, got 4"), "{err}");

    let not_hex = "zz".repeat(KEY_LEN);
    let err = master_key_from_env(Some(&not_hex), None, file_must_not_be_read)
        .expect_err("non-hex is rejected");
    assert!(err.contains("not valid hex"), "{err}");
    assert!(
        !err.contains(&not_hex),
        "error must not echo the value: {err}"
    );

    // Right character count, wrong bytes: must be rejected, not panic in the
    // byte-sliced hex decoder.
    let non_ascii = "é".repeat(KEY_LEN * 2);
    let err = master_key_from_env(Some(&non_ascii), None, file_must_not_be_read)
        .expect_err("non-ASCII is rejected");
    assert!(err.contains("not valid hex"), "{err}");

    // The same validation applies to a file's contents.
    let err = master_key_from_env(None, Some("/run/secrets/key"), |_| Ok("0123".to_string()))
        .expect_err("short file contents are rejected");
    assert!(
        err.contains(MASTER_KEY_FILE_ENV) && err.contains("got 4"),
        "{err}"
    );
}

#[test]
fn empty_env_values_fall_through_to_the_keychain() {
    assert!(master_key_from_env(None, None, file_must_not_be_read)
        .expect("nothing set is fine")
        .is_none());
    assert!(
        master_key_from_env(Some(""), Some("   "), file_must_not_be_read)
            .expect("empty values count as unset")
            .is_none()
    );
}

#[test]
fn read_master_key_file_returns_contents_and_reports_a_missing_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("master.key");
    std::fs::write(&path, format!("{}\n", hex_key(0x33))).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    let contents = read_master_key_file(&path).expect("readable file");
    assert_eq!(contents.trim(), hex_key(0x33));

    let err = read_master_key_file(&tmp.path().join("missing.key")).expect_err("missing file");
    assert!(
        err.contains("cannot inspect master key file permissions"),
        "{err}"
    );
}

#[cfg(unix)]
#[test]
fn read_master_key_file_accepts_read_only_secret_mounts() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("master.key");
    std::fs::write(&path, hex_key(0x44)).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

    read_master_key_file(&path).expect("read-only secret mounts are supported");
}

#[cfg(unix)]
#[test]
fn read_master_key_file_rejects_other_writable_files() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("master.key");
    std::fs::write(&path, hex_key(0x45)).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o602)).unwrap();

    let err = read_master_key_file(&path).expect_err("other-writable key files are unsafe");
    assert!(err.contains("writable by other users"), "{err}");
}

#[cfg(unix)]
#[test]
fn key_file_permissions_reject_other_writes_but_allow_read_only_access() {
    for mode in [0o400, 0o600, 0o644, 0o444, 0o604, 0o440, 0o640] {
        assert!(
            !key_source::key_file_mode_is_other_writable(mode),
            "{mode:04o}"
        );
    }
    for mode in [0o602, 0o666] {
        assert!(
            key_source::key_file_mode_is_other_writable(mode),
            "{mode:04o}"
        );
    }
}

#[test]
fn parse_master_key_hex_round_trips_an_encoded_key() {
    let key_bytes = crypto::generate_key().expect("key generation succeeds");
    let encoded = crypto::key_to_hex(&key_bytes);
    let parsed = parse_master_key_hex(&encoded).expect("valid");
    assert_eq!(parsed.as_slice(), key_bytes.as_slice());
}

#[test]
fn env_value_distinguishes_unset_from_invalid_unicode() {
    use std::env::VarError;

    assert_eq!(
        env_value(MASTER_KEY_ENV, Ok("abc".to_string())).expect("set"),
        Some("abc".to_string())
    );
    assert_eq!(
        env_value(MASTER_KEY_ENV, Err(VarError::NotPresent)).expect("unset is not an error"),
        None
    );

    // Invalid Unicode must be rejected, not treated as unset: falling through
    // to the keychain could mint a different key and orphan `secrets.enc`.
    let rejected = std::ffi::OsString::from("not-the-real-bytes");
    let err = env_value(MASTER_KEY_FILE_ENV, Err(VarError::NotUnicode(rejected)))
        .expect_err("invalid Unicode is a configuration error");
    assert!(err.contains(MASTER_KEY_FILE_ENV), "{err}");
    assert!(err.contains("not valid Unicode"), "{err}");
    assert!(
        !err.contains("not-the-real-bytes"),
        "error must not echo the value: {err}"
    );
}

// ── Wiring: the real env → `try_load_master_key` → key, no keychain ─────────
//
// `try_load_master_key` is uncached: it reads the environment on every call
// and never touches the process-wide `MASTER_KEY` (only `init_master_key`
// does, and no test here calls it; the one-time logic is tested through
// `init_once` with a local `OnceLock` below). Each test mutates the env only
// under `EnvVarGuard::locked()`, which serializes on the crate's shared
// `TEST_ENV_LOCK` and restores the previous values before releasing it, so
// the tests cannot observe each other's values in any order.

#[test]
fn try_load_master_key_prefers_the_inline_env_key_and_never_touches_the_keychain() {
    let hex = hex_key(0x55);
    let _env = crate::config::test_env::EnvVarGuard::locked()
        .with(MASTER_KEY_ENV, &hex)
        .without(MASTER_KEY_FILE_ENV);

    // A real OS keychain cannot be exercised under `cargo test` (the first
    // access blocks on a GUI prompt), so reaching the keychain here would hang
    // or fail; returning means the env source short-circuited it.
    let (key, source) = try_load_master_key().expect("inline env key loads");
    assert_eq!(key, [0x55u8; KEY_LEN]);
    assert_eq!(source, MASTER_KEY_ENV);
}

#[test]
fn try_load_master_key_reads_the_file_source_and_is_stable_across_restarts() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("master.key");
    std::fs::write(&path, format!("{}\n", hex_key(0x66))).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let _env = crate::config::test_env::EnvVarGuard::locked()
        .without(MASTER_KEY_ENV)
        .with(MASTER_KEY_FILE_ENV, &path);

    // Two loads stand in for two process starts: the same file must yield
    // the same key, so a secret encrypted before a restart decrypts after it.
    let (first, source) = try_load_master_key().expect("file key loads");
    let (second, _) = try_load_master_key().expect("file key loads again");
    assert_eq!(first, [0x66u8; KEY_LEN]);
    assert_eq!(first, second);
    assert_eq!(source, MASTER_KEY_FILE_ENV);

    let blob = crypto::encrypt(&first, b"sk-live-secret").expect("encrypt");
    assert_eq!(
        crypto::decrypt(&second, &blob)
            .expect("decrypt with the restarted key")
            .as_slice(),
        b"sk-live-secret"
    );
}

#[test]
fn try_load_master_key_reports_a_configured_source_error_as_configured() {
    let _env = crate::config::test_env::EnvVarGuard::locked()
        .with(MASTER_KEY_ENV, "not-a-key")
        .without(MASTER_KEY_FILE_ENV);

    match try_load_master_key() {
        Err(MasterKeyError::Configured(e)) => {
            assert!(e.contains(MASTER_KEY_ENV), "{e}");
            assert!(!e.contains("not-a-key"), "must not echo the value: {e}");
        }
        other => panic!("expected a configuration error, got {other:?}"),
    }
}

#[test]
fn a_secret_written_under_a_configured_key_reads_back_after_a_restart() {
    let workspace = secrets_tempdir();
    let key_dir = secrets_tempdir();
    let key_path = key_dir.path().join("master.key");
    std::fs::write(&key_path, format!("{}\n", hex_key(0x88))).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let _env = crate::config::test_env::EnvVarGuard::locked()
        .without(MASTER_KEY_ENV)
        .with(MASTER_KEY_FILE_ENV, &key_path);

    // First "process": load the configured key and store a secret through the
    // same lock → read → encrypt → write path `SecretBackend::set` uses.
    // `get`/`set` read the key from the process-wide `MASTER_KEY`, which
    // this test must not populate, so the key is passed explicitly.
    {
        let (key, _) = try_load_master_key().expect("configured key loads");
        let backend = EncryptedFileBackend::new(workspace.path());
        backend
            .set_with_key(&key, "provider:openai", "sk-live-secret")
            .expect("store under the configured key");
    }
    let on_disk = std::fs::read(workspace.path().join(SECRETS_FILENAME)).unwrap();
    assert!(
        !on_disk
            .windows(b"sk-live-secret".len())
            .any(|w| w == b"sk-live-secret"),
        "secrets.enc must not hold the plaintext"
    );

    // Second "process": a fresh key load and a fresh backend over the same
    // workspace read the secret back.
    let (key, _) = try_load_master_key().expect("configured key loads after restart");
    let backend = EncryptedFileBackend::new(workspace.path());
    assert_eq!(
        backend
            .get_with_key(&key, "provider:openai")
            .expect("read after restart")
            .as_deref(),
        Some("sk-live-secret")
    );
}

// ── One-time init keeps a configuration error ───────────────────────────────

#[test]
fn init_once_returns_the_configuration_error_on_every_call() {
    let cell = OnceLock::new();
    let first = init_once(&cell, || {
        Err(format!(
            "{MASTER_KEY_ENV}: expected 64 hex characters, got 4"
        ))
    });
    assert!(first.is_err(), "first call reports the error");

    // `OnceLock` does not rerun the closure; the cached error must still
    // surface rather than a silent `Ok` with no key loaded.
    let second = init_once(&cell, || panic!("init must run at most once"));
    assert_eq!(second, first);
}

#[test]
fn init_once_caches_a_loaded_key_and_a_keychain_outage_as_ok() {
    let loaded = OnceLock::new();
    assert_eq!(init_once(&loaded, || Ok(Some([0x99u8; KEY_LEN]))), Ok(()));
    assert_eq!(
        init_once(&loaded, || panic!("init must run at most once")),
        Ok(())
    );
    assert_eq!(loaded.get(), Some(&Ok(Some([0x99u8; KEY_LEN]))));

    // No key (non-encrypted backend, or the #3311 keychain outage) is not a
    // startup error.
    let unavailable = OnceLock::new();
    assert_eq!(init_once(&unavailable, || Ok(None)), Ok(()));
    assert_eq!(
        init_once(&unavailable, || panic!("init must run at most once")),
        Ok(())
    );
}
