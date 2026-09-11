//! Shared ChaCha20-Poly1305 cryptographic helpers.
//!
//! Used by both [`super::encrypted_store::SecretStore`] (config field encryption)
//! and [`super::encrypted_file_backend::EncryptedFileBackend`] (secrets file encryption).

use chacha20poly1305::aead::{Aead, KeyInit, OsRng};
use chacha20poly1305::{AeadCore, ChaCha20Poly1305, Key, Nonce};

pub(super) const NONCE_LEN: usize = 12;
pub(super) const KEY_LEN: usize = 32;

/// Encrypt `plaintext` with ChaCha20-Poly1305. Returns `nonce || ciphertext || tag`.
pub(super) fn chacha20_encrypt(key: &[u8; KEY_LEN], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| format!("ChaCha20 encryption failed: {e}"))?;

    let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&ciphertext);
    Ok(blob)
}

/// Decrypt a `nonce || ciphertext || tag` blob produced by [`chacha20_encrypt`].
pub(super) fn chacha20_decrypt(key: &[u8; KEY_LEN], blob: &[u8]) -> Result<Vec<u8>, String> {
    if blob.len() <= NONCE_LEN {
        return Err("encrypted blob too short (missing nonce)".to_string());
    }
    let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
    let nonce = Nonce::from_slice(nonce_bytes);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    cipher
        .decrypt(nonce, ciphertext)
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
