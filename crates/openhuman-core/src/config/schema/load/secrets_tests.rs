use super::*;

#[tokio::test]
async fn config_secret_encryption_fails_closed_without_tinysecurity() {
    #[cfg(feature = "security-module")]
    let _absent = crate::modules::security::test_host::absent();
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.config_path = dir.path().join("config.toml");
    config.modules.enabled = false;
    config.secrets.encrypt = true;
    config.api_key = Some("must-not-be-written-in-plaintext".into());

    let error = encrypt_config_secrets(&mut config)
        .await
        .expect_err("config save must require TinySecurity cipher operations");

    // `to_string` is the outermost context only; the cause is in the chain.
    assert!(
        format!("{error:#}").contains("security module unavailable"),
        "{error:#}"
    );
    assert!(!dir.path().join(".secret_key").exists());
    assert_eq!(
        config.api_key.as_deref(),
        Some("must-not-be-written-in-plaintext")
    );
}

#[tokio::test]
async fn failed_storage_url_decryption_keeps_the_sealed_value() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.config_path = dir.path().join("config.toml");
    config.modules.enabled = false;
    config.secrets.encrypt = true;
    let sealed = format!("enc2:{}", "00".repeat(32));
    config.storage.url = Some(sealed.clone());

    assert!(!decrypt_config_secrets(&mut config, dir.path())
        .await
        .unwrap());
    assert_eq!(config.storage.url.as_deref(), Some(sealed.as_str()));
}

#[tokio::test]
async fn disabled_config_secret_encryption_is_a_plaintext_noop() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.config_path = dir.path().join("config.toml");
    config.secrets.encrypt = false;
    config.api_key = Some("plaintext-key".into());

    encrypt_config_secrets(&mut config).await.unwrap();
    assert_eq!(config.api_key.as_deref(), Some("plaintext-key"));
}

/// The Settings save RPC reports `config_save_error`, which prints only the
/// outermost context. It must still carry why sealing failed, not a bare
/// "Failed to encrypt api_key".
#[tokio::test]
async fn a_failed_secret_encryption_reports_the_field_and_the_reason() {
    let dir = tempfile::tempdir().unwrap();
    // A store without its configuration cannot reach TinySecurity.
    let store = crate::security::secret_storage::SecretStore::new(dir.path(), true);
    let mut value = Some("sk-must-not-leak".to_string());

    let error = encrypt_optional_secret(&store, &mut value, "api_key")
        .await
        .expect_err("sealing without TinySecurity must fail");
    let shown = crate::config::ops::config_save_error(error);

    assert!(shown.starts_with("Failed to encrypt api_key: "), "{shown}");
    assert!(
        shown.contains("TinySecurity configuration is unavailable"),
        "{shown}"
    );
    assert!(!shown.contains("sk-must-not-leak"), "{shown}");
}
