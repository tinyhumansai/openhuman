use super::*;

#[tokio::test]
async fn config_secret_encryption_fails_closed_without_tinysecurity() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.config_path = dir.path().join("config.toml");
    config.modules.enabled = false;
    config.secrets.encrypt = true;
    config.api_key = Some("must-not-be-written-in-plaintext".into());

    let error = encrypt_config_secrets(&mut config)
        .await
        .expect_err("config save must require TinySecurity cipher operations");

    assert!(error.to_string().contains("unavailable"));
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
