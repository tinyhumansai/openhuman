//! Encrypted-file corruption and import policy over the secret-store port.
use super::*;
use crate::security::secret_storage::encrypted_file_backend::EncryptedFileBackend;
use std::collections::HashMap;
use tinystoragedrivers::secrets::crypto;
// ── EncryptedFileBackend ─────────────────────────────────────────────────────

const KEY: [u8; 32] = [7; 32];

fn secrets_tempdir() -> tempfile::TempDir {
    #[cfg(target_os = "macos")]
    {
        tempfile::tempdir_in("/private/tmp").expect("canonical macOS temp directory")
    }
    #[cfg(not(target_os = "macos"))]
    {
        tempfile::tempdir().expect("tempdir")
    }
}

fn write_private_file(path: &std::path::Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}

#[test]
fn file_backend_round_trips_and_deletes() {
    let dir = secrets_tempdir();
    let b = EncryptedFileBackend::new(dir.path());
    assert_eq!(b.get_with_key(&KEY, "u:a").unwrap(), None);
    b.set_with_key(&KEY, "u:a", "1").unwrap();
    b.set_with_key(&KEY, "u:b", "2").unwrap();
    assert_eq!(b.get_with_key(&KEY, "u:a").unwrap().as_deref(), Some("1"));
    let blob = std::fs::read(dir.path().join("secrets.enc")).unwrap();
    let map: HashMap<String, String> =
        serde_json::from_slice(&crypto::decrypt(&KEY, &blob).unwrap()).unwrap();
    assert_eq!(map.len(), 2, "one blob over a json object of strings");
}

fn quarantined(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|name| name.starts_with("secrets.enc.corrupt."))
        .collect()
}

#[test]
fn a_file_under_another_key_is_quarantined_and_the_store_recovers() {
    let dir = secrets_tempdir();
    let b = EncryptedFileBackend::new(dir.path());
    b.set_with_key(&[9; 32], "u:old", "old-value").unwrap();
    let original = std::fs::read(dir.path().join("secrets.enc")).unwrap();

    // Desktop policy, unlike the driver's fail-closed: log, move aside, go on.
    assert_eq!(b.get_with_key(&KEY, "u:old").unwrap(), None);
    let moved = quarantined(dir.path());
    assert_eq!(moved.len(), 1, "{moved:?}");
    assert_eq!(
        std::fs::read(dir.path().join(&moved[0])).unwrap(),
        original,
        "the bytes are preserved, not deleted"
    );
    b.set_with_key(&KEY, "u:new", "fresh").unwrap();
    assert_eq!(
        b.get_with_key(&KEY, "u:new").unwrap().as_deref(),
        Some("fresh")
    );
}

#[test]
fn a_garbage_file_does_not_wedge_writes() {
    let dir = secrets_tempdir();
    write_private_file(
        &dir.path().join("secrets.enc"),
        b"definitely not a blob at all, no",
    );
    let b = EncryptedFileBackend::new(dir.path());
    b.set_with_key(&KEY, "u:a", "1").unwrap();
    assert_eq!(b.get_with_key(&KEY, "u:a").unwrap().as_deref(), Some("1"));
    assert_eq!(quarantined(dir.path()).len(), 1);
}

#[test]
fn an_undecodable_payload_is_quarantined_too() {
    // Decrypts, but is not a json object of strings.
    let dir = secrets_tempdir();
    let blob = crypto::encrypt(&KEY, b"[1,2,3]").unwrap();
    write_private_file(&dir.path().join("secrets.enc"), &blob);
    let b = EncryptedFileBackend::new(dir.path());
    assert_eq!(b.get_with_key(&KEY, "u:a").unwrap(), None);
    assert_eq!(quarantined(dir.path()).len(), 1);
}

#[test]
fn a_legacy_dev_keychain_is_imported_once() {
    let dir = secrets_tempdir();
    std::fs::write(
        dir.path().join("dev-keychain.json"),
        br#"{"u:legacy":"from-json"}"#,
    )
    .unwrap();
    let b = EncryptedFileBackend::new(dir.path());
    assert_eq!(
        b.get_with_key(&KEY, "u:legacy").unwrap().as_deref(),
        Some("from-json")
    );
    assert!(!dir.path().join("dev-keychain.json").exists());
    assert!(dir.path().join("dev-keychain.json.migrated").exists());
    assert!(dir.path().join("secrets.enc").exists());
}

#[test]
fn recovery_never_quarantines_a_file_that_is_not_confirmed_corrupt() {
    // The re-probe fails with an I/O error (the path is a directory), not a
    // crypto/serialization one: nothing says the contents are bad, so nothing
    // is moved and the error is surfaced.
    let dir = secrets_tempdir();
    let path = dir.path().join("secrets.enc");
    std::fs::create_dir(&path).unwrap();
    let cause = tinystoragedrivers::StorageError::crypto("earlier failure");
    let error = recover_corrupt_file(&path, &KEY, &cause).unwrap_err();
    assert!(matches!(error, SecretStorageError::Backend(_)), "{error:?}");
    assert!(path.is_dir(), "left in place");
}

#[test]
fn recovery_leaves_a_file_that_became_readable() {
    let dir = secrets_tempdir();
    let b = EncryptedFileBackend::new(dir.path());
    b.set_with_key(&KEY, "u:a", "1").unwrap();
    let cause = tinystoragedrivers::StorageError::crypto("stale failure");
    recover_corrupt_file(&dir.path().join("secrets.enc"), &KEY, &cause).unwrap();
    assert_eq!(b.get_with_key(&KEY, "u:a").unwrap().as_deref(), Some("1"));
}

#[test]
fn recovery_does_not_quarantine_a_valid_file_replaced_after_the_probe() {
    let dir = secrets_tempdir();
    let backend = EncryptedFileBackend::new(dir.path());
    let path = dir.path().join("secrets.enc");
    backend.set_with_key(&KEY, "u:a", "1").unwrap();
    write_private_file(&path, b"corrupt before probe");
    let valid_map = HashMap::from([("u:a".to_owned(), "1".to_owned())]);
    let valid_blob = crypto::encrypt(&KEY, &serde_json::to_vec(&valid_map).unwrap()).unwrap();
    let cause = tinystoragedrivers::StorageError::crypto("stale failure");

    recover_corrupt_file_after_probe(&path, &KEY, &cause, || {
        write_private_file(&path, &valid_blob);
    })
    .unwrap();

    assert_eq!(
        backend.get_with_key(&KEY, "u:a").unwrap().as_deref(),
        Some("1")
    );
    assert!(quarantined(dir.path()).is_empty());
}
