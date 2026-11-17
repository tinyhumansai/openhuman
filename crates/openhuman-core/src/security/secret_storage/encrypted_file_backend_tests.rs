use super::*;
use crate::security::secret_storage::file_store;
use std::collections::HashMap;

pub(super) fn secrets_tempdir() -> tempfile::TempDir {
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
    #[cfg(feature = "security-module")]
    let _absent = crate::modules::security::test_host::absent();
    let _env = crate::config::test_env::EnvVarGuard::locked_unset_many(&[
        MASTER_KEY_ENV,
        MASTER_KEY_FILE_ENV,
    ]);
    let workspace = secrets_tempdir();
    let backend = EncryptedFileBackend::new(workspace.path());

    let error = backend
        .get_profile_read_only("account:session")
        .unwrap_err();

    assert!(
        error.to_string().contains("master key unavailable"),
        "{error}"
    );
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
    assert!(
        !legacy_path.exists(),
        "{:?}",
        std::fs::read_dir(dir.path())
            .map(|d| d.flatten().map(|e| e.file_name()).collect::<Vec<_>>())
    );
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
    // Seed the store before the plaintext exists: any backend call runs the
    // import first, so writing through it with the source present would
    // complete the migration instead of setting up the interrupted state. An
    // earlier attempt replayed "a:import"; a different "z:conflict" landed since.
    backend.set_with_key(&key, "a:import", "imported").unwrap();
    backend
        .set_with_key(&key, "z:conflict", "different")
        .unwrap();
    write_private_file(&legacy_path, &source_bytes);

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

/// Reproduces an install that ran the earlier import order: the source was
/// staged, every entry was replayed into `secrets.enc`, and cleanup refused
/// because the destination object changed after the receipt was issued. The
/// staged payload, the replayed entries and the plaintext are all on disk, but
/// the receipt died with that process.
fn stage_an_import_whose_cleanup_was_refused(
    backend: &EncryptedFileBackend,
    key: &[u8; KEY_LEN],
    legacy_path: &Path,
    entries: &HashMap<String, String>,
) -> Vec<u8> {
    use tinystoragedrivers::secrets::{FileImportSource, SecretStore as _};
    // Seed before the plaintext exists: a backend call with the source
    // present would run the import instead of setting up the stuck state.
    for (name, value) in entries {
        backend.set_with_key(key, name, value).unwrap();
    }
    let bytes = serde_json::to_vec(entries).unwrap();
    write_private_file(legacy_path, &bytes);
    let store = std::sync::Arc::new(backend.store(key));
    let source = FileImportSource::open(legacy_path).unwrap();
    let staged = bytes.clone();
    crate::storage::block_on(async move {
        store
            .prepare_file_import(
                LEGACY_IMPORT_OPERATION,
                LEGACY_IMPORT_PAYLOAD,
                &staged,
                &source,
            )
            .await
            .map(drop)
    })
    .unwrap();
    bytes
}

#[test]
fn a_staged_import_left_by_a_refused_cleanup_removes_the_verified_plaintext() {
    let dir = secrets_tempdir();
    let backend = EncryptedFileBackend::new(dir.path());
    let key = [0x42; KEY_LEN];
    let legacy_path = dir.path().join(LEGACY_DEV_KEYCHAIN);
    let entries = HashMap::from([
        ("user:token".to_string(), "secret-value".to_string()),
        ("user:second".to_string(), "second-value".to_string()),
    ]);
    stage_an_import_whose_cleanup_was_refused(&backend, &key, &legacy_path, &entries);

    backend.import_legacy_dev_keychain(&key).unwrap();

    assert!(
        !legacy_path.exists(),
        "a later start must remove plaintext every entry of which is verified encrypted"
    );
    for (name, value) in &entries {
        assert_eq!(
            backend.get_with_key(&key, name).unwrap().as_deref(),
            Some(value.as_str())
        );
    }
    assert_eq!(
        backend
            .get_with_key(&key, LEGACY_IMPORT_COMPLETE)
            .unwrap()
            .as_deref(),
        Some("1")
    );
}

#[test]
fn a_staged_import_keeps_plaintext_that_differs_from_its_staged_backup() {
    let dir = secrets_tempdir();
    let backend = EncryptedFileBackend::new(dir.path());
    let key = [0x42; KEY_LEN];
    let legacy_path = dir.path().join(LEGACY_DEV_KEYCHAIN);
    let entries = HashMap::from([("user:token".to_string(), "secret-value".to_string())]);
    stage_an_import_whose_cleanup_was_refused(&backend, &key, &legacy_path, &entries);
    // The plaintext gained an entry the encrypted store never received.
    let edited = serde_json::to_vec(&HashMap::from([
        ("user:token".to_string(), "secret-value".to_string()),
        ("user:new".to_string(), "unimported".to_string()),
    ]))
    .unwrap();
    write_private_file(&legacy_path, &edited);

    assert!(backend.import_legacy_dev_keychain(&key).is_err());
    assert_eq!(std::fs::read(&legacy_path).unwrap(), edited);
    assert_eq!(
        backend.get_with_key(&key, LEGACY_IMPORT_COMPLETE).unwrap(),
        None
    );
}
