use super::*;
use tempfile::TempDir;

#[tokio::test]
async fn encrypted_operations_fail_closed_without_tinysecurity() {
    let temp = TempDir::new().unwrap();
    let mut config = crate::config::Config::default();
    config.modules.enabled = false;
    let store = SecretStore::new_with_config(temp.path(), true, &config);

    let error = store
        .encrypt_via_security("must-not-use-host-crypto")
        .await
        .expect_err("enabled encryption requires TinySecurity");

    assert!(error.to_string().contains("unavailable"));
}

#[tokio::test]
async fn existing_key_decryption_fails_closed_without_tinysecurity() {
    let temp = TempDir::new().unwrap();
    let mut config = crate::config::Config::default();
    config.modules.enabled = false;
    let store = SecretStore::new_with_config(temp.path(), true, &config);

    let error = store
        .decrypt_with_existing_key_via_security(
            &format!("enc2:{}", "00".repeat(32)),
            &"ab".repeat(32),
        )
        .await
        .expect_err("explicit-key decryption requires TinySecurity");

    assert!(error.to_string().contains("unavailable"));
}

#[tokio::test]
async fn disabled_encryption_preserves_plaintext() {
    let temp = TempDir::new().unwrap();
    let store = SecretStore::new(temp.path(), false);

    assert_eq!(store.encrypt_via_security("plain").await.unwrap(), "plain");
    assert_eq!(store.decrypt_via_security("plain").await.unwrap(), "plain");
}

#[test]
fn encrypted_prefixes_remain_compatible() {
    assert!(SecretStore::is_encrypted("enc2:ciphertext"));
    assert!(SecretStore::is_encrypted("enc:legacy"));
    assert!(SecretStore::needs_migration("enc:legacy"));
    assert!(!SecretStore::needs_migration("enc2:ciphertext"));
    assert!(!SecretStore::is_encrypted("plaintext"));
}

#[test]
fn keyring_user_id_uses_directory_name_or_stable_hash() {
    assert_eq!(keyring_user_id_from_dir(Path::new("/users/alice")), "alice");
    let first = keyring_user_id_from_dir(Path::new("/"));
    assert_eq!(first, keyring_user_id_from_dir(Path::new("/")));
    assert!(first.starts_with("secretstore-path-"));
}
