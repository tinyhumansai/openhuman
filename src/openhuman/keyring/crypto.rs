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
    if hex.len() % 2 != 0 {
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
mod tests {
    use super::*;

    #[test]
    fn roundtrip_encrypt_decrypt() {
        let key = [42u8; KEY_LEN];
        let plaintext = b"hello world";
        let blob = chacha20_encrypt(&key, plaintext).unwrap();
        let decrypted = chacha20_decrypt(&key, &blob).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn decrypt_wrong_key_fails() {
        let key1 = [1u8; KEY_LEN];
        let key2 = [2u8; KEY_LEN];
        let blob = chacha20_encrypt(&key1, b"secret").unwrap();
        assert!(chacha20_decrypt(&key2, &blob).is_err());
    }

    #[test]
    fn decrypt_short_blob_fails() {
        let key = [0u8; KEY_LEN];
        assert!(chacha20_decrypt(&key, &[0u8; NONCE_LEN]).is_err());
    }

    #[test]
    fn hex_roundtrip() {
        let data = vec![0xde, 0xad, 0xbe, 0xef];
        assert_eq!(hex_encode(&data), "deadbeef");
        assert_eq!(hex_decode("deadbeef").unwrap(), data);
    }
}
