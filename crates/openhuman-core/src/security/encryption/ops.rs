//! JSON-RPC / CLI controller surface for encryption-focused helpers.

use crate::config::Config;
use crate::core::Outcome;

pub async fn encrypt_secret(config: &Config, plaintext: &str) -> Result<Outcome<String>, String> {
    crate::credentials::ops::encrypt_secret(config, plaintext).await
}

pub async fn decrypt_secret(config: &Config, ciphertext: &str) -> Result<Outcome<String>, String> {
    crate::credentials::ops::decrypt_secret(config, ciphertext).await
}
