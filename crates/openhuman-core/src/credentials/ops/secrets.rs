//! Encrypt / decrypt arbitrary secrets with the config's `SecretStore`.

use crate::config::Config;
use crate::core::Outcome;
use crate::security::secret_storage::SecretStore;

pub(crate) fn secret_store_for_config(config: &Config) -> SecretStore {
    let data_dir = config
        .config_path
        .parent()
        .map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from);
    SecretStore::new_with_config(&data_dir, true, config)
}

pub async fn encrypt_secret(config: &Config, plaintext: &str) -> Result<Outcome<String>, String> {
    let store = secret_store_for_config(config);
    let ciphertext = store
        .encrypt_via_security(plaintext)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Outcome::single_log(ciphertext, "secret encrypted"))
}

pub async fn decrypt_secret(config: &Config, ciphertext: &str) -> Result<Outcome<String>, String> {
    let store = secret_store_for_config(config);
    let plaintext = store
        .decrypt_via_security(ciphertext)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Outcome::single_log(plaintext, "secret decrypted"))
}
