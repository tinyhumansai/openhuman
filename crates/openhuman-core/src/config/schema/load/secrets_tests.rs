use super::*;

#[test]
fn gemini_key_round_trips_with_encryption_enabled() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.config_path = dir.path().join("config.toml");
    config.secrets.encrypt = true;
    config.search.gemini.api_key = Some("gemini-sentinel".into());

    encrypt_config_secrets(&mut config).unwrap();
    let ciphertext = config.search.gemini.api_key.as_deref().unwrap();
    assert!(ciphertext.starts_with("enc2:"));
    assert!(!ciphertext.contains("gemini-sentinel"));
    assert!(!decrypt_config_secrets(&mut config, dir.path()).unwrap());
    assert_eq!(
        config.search.gemini.api_key.as_deref(),
        Some("gemini-sentinel")
    );
}
