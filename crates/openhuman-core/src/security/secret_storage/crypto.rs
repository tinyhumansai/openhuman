//! Shared ChaCha20-Poly1305 cryptographic helpers.
//!
//! Used by both [`super::encrypted_store::SecretStore`] (config field encryption)
//! and [`super::encrypted_file_backend::EncryptedFileBackend`] (secrets file encryption).

use chacha20poly1305::aead::OsRng;
use tinystoragedrivers::secrets::crypto as secrets_crypto;

pub(super) const NONCE_LEN: usize = 12;
pub(super) const KEY_LEN: usize = 32;

/// Encrypt `plaintext` with ChaCha20-Poly1305. Returns `nonce || ciphertext || tag`.
///
/// The cipher is `tinystoragedrivers`' (`secrets::crypto`), the same format
/// its secret stores read and write.
pub(super) fn chacha20_encrypt(key: &[u8; KEY_LEN], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    secrets_crypto::encrypt(key, plaintext).map_err(|e| format!("ChaCha20 encryption failed: {e}"))
}

/// Decrypt a `nonce || ciphertext || tag` blob produced by [`chacha20_encrypt`].
pub(super) fn chacha20_decrypt(key: &[u8; KEY_LEN], blob: &[u8]) -> Result<Vec<u8>, String> {
    if blob.len() <= NONCE_LEN {
        return Err("encrypted blob too short (missing nonce)".to_string());
    }
    secrets_crypto::decrypt(key, blob)
        .map(|plaintext| plaintext.to_vec())
        .map_err(|_| "decryption failed — wrong key or tampered data".to_string())
}

/// Generate `len` cryptographically random bytes.
pub(super) fn generate_random_bytes(len: usize) -> Vec<u8> {
    use chacha20poly1305::aead::rand_core::RngCore;
    let mut bytes = vec![0u8; len];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

/// Hex-encode bytes (lowercase).
pub(super) fn hex_encode(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

/// Decode a hex string into bytes.
pub(super) fn hex_decode(hex: &str) -> Result<Vec<u8>, String> {
    if !hex.len().is_multiple_of(2) {
        return Err("hex string has odd length".to_string());
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .map_err(|e| format!("invalid hex at position {i}: {e}"))
        })
        .collect()
}

#[cfg(test)]
#[path = "crypto_tests.rs"]
mod tests;
