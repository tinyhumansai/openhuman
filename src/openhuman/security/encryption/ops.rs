//! JSON-RPC / CLI controller surface for encryption-focused helpers.

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

pub async fn encrypt_secret(
    config: &Config,
    plaintext: &str,
) -> Result<RpcOutcome<String>, String> {
    crate::openhuman::security::credentials::rpc::encrypt_secret(config, plaintext).await
}

pub async fn decrypt_secret(
    config: &Config,
    ciphertext: &str,
) -> Result<RpcOutcome<String>, String> {
    crate::openhuman::security::credentials::rpc::decrypt_secret(config, ciphertext).await
}
