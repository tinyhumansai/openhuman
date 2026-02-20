//! AES-256-GCM encryption layer for AI memory storage.
//!
//! All memory data (SQLite content, embeddings, session transcripts) is
//! encrypted at rest using AES-256-GCM. Keys are derived from a user
//! password via Argon2id.

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use argon2::{self, Algorithm, Argon2, Params, Version};
use aes_gcm::aead::rand_core::RngCore;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Salt length for Argon2id key derivation
const SALT_LENGTH: usize = 16;
/// Nonce length for AES-256-GCM (96 bits)
const NONCE_LENGTH: usize = 12;
/// Derived key length (256 bits for AES-256)
const KEY_LENGTH: usize = 32;

/// Encrypted payload with metadata for decryption
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EncryptedPayload {
    /// AES-256-GCM ciphertext
    pub ciphertext: Vec<u8>,
    /// Random nonce used for this encryption
    pub nonce: Vec<u8>,
    /// Argon2id salt used for key derivation
    pub salt: Vec<u8>,
}

/// Encryption key material
#[derive(Clone)]
pub struct EncryptionKey {
    key_bytes: [u8; KEY_LENGTH],
}

impl EncryptionKey {
    /// Derive an encryption key from a password and salt using Argon2id.
    pub fn derive(password: &str, salt: &[u8]) -> Result<Self, String> {
        let params = Params::new(65536, 3, 1, Some(KEY_LENGTH))
            .map_err(|e| format!("Argon2 params error: {e}"))?;
        let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

        let mut key_bytes = [0u8; KEY_LENGTH];
        argon2
            .hash_password_into(password.as_bytes(), salt, &mut key_bytes)
            .map_err(|e| format!("Key derivation failed: {e}"))?;

        Ok(Self { key_bytes })
    }

    /// Generate a new random salt for key derivation.
    pub fn generate_salt() -> Vec<u8> {
        let mut salt = vec![0u8; SALT_LENGTH];
        OsRng.fill_bytes(&mut salt);
        salt
    }

    /// Encrypt plaintext bytes.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<EncryptedPayload, String> {
        let cipher =
            Aes256Gcm::new_from_slice(&self.key_bytes).map_err(|e| format!("Cipher init: {e}"))?;

        let mut nonce_bytes = [0u8; NONCE_LENGTH];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| format!("Encryption failed: {e}"))?;

        Ok(EncryptedPayload {
            ciphertext,
            nonce: nonce_bytes.to_vec(),
            salt: Vec::new(), // Salt is stored separately in the key file
        })
    }

    /// Decrypt an encrypted payload.
    pub fn decrypt(&self, payload: &EncryptedPayload) -> Result<Vec<u8>, String> {
        let cipher =
            Aes256Gcm::new_from_slice(&self.key_bytes).map_err(|e| format!("Cipher init: {e}"))?;

        let nonce = Nonce::from_slice(&payload.nonce);

        cipher
            .decrypt(nonce, payload.ciphertext.as_ref())
            .map_err(|e| format!("Decryption failed: {e}"))
    }

    /// Encrypt a string and return base64-encoded JSON payload.
    pub fn encrypt_string(&self, plaintext: &str) -> Result<String, String> {
        let payload = self.encrypt(plaintext.as_bytes())?;
        serde_json::to_string(&payload).map_err(|e| format!("Serialization failed: {e}"))
    }

    /// Decrypt a base64-encoded JSON payload back to a string.
    pub fn decrypt_string(&self, encrypted_json: &str) -> Result<String, String> {
        let payload: EncryptedPayload =
            serde_json::from_str(encrypted_json).map_err(|e| format!("Deserialization: {e}"))?;
        let plaintext = self.decrypt(&payload)?;
        String::from_utf8(plaintext).map_err(|e| format!("UTF-8 decode: {e}"))
    }
}

/// Get the path to the AlphaHuman data directory (~/.alphahuman/).
pub fn get_data_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "Cannot determine home directory".to_string())?;
    let data_dir = home.join(".alphahuman");
    std::fs::create_dir_all(&data_dir)
        .map_err(|e| format!("Failed to create data directory: {e}"))?;
    Ok(data_dir)
}

/// Get the path to the encryption key file (~/.alphahuman/encryption.key).
fn get_key_file_path() -> Result<PathBuf, String> {
    Ok(get_data_dir()?.join("encryption.key"))
}

/// Key file stores the salt; the actual key is derived at runtime from password.
#[derive(Serialize, Deserialize)]
struct KeyFile {
    salt: Vec<u8>,
    /// Version for future key rotation
    version: u32,
}

// --- Tauri Commands ---

/// Initialize encryption with a password. Creates key file if needed.
#[tauri::command]
pub async fn ai_init_encryption(password: String) -> Result<bool, String> {
    let key_path = get_key_file_path()?;

    if key_path.exists() {
        // Key file exists, verify password works by loading it
        let content =
            std::fs::read_to_string(&key_path).map_err(|e| format!("Read key file: {e}"))?;
        let key_file: KeyFile =
            serde_json::from_str(&content).map_err(|e| format!("Parse key file: {e}"))?;
        let _key = EncryptionKey::derive(&password, &key_file.salt)?;
        Ok(true)
    } else {
        // Create new key file with random salt
        let salt = EncryptionKey::generate_salt();
        let key_file = KeyFile { salt, version: 1 };
        let content =
            serde_json::to_string_pretty(&key_file).map_err(|e| format!("Serialize: {e}"))?;
        std::fs::write(&key_path, content).map_err(|e| format!("Write key file: {e}"))?;
        Ok(true)
    }
}

/// Encrypt a string value using the password-derived key.
#[tauri::command]
pub async fn ai_encrypt(password: String, plaintext: String) -> Result<String, String> {
    let key_path = get_key_file_path()?;
    let content = std::fs::read_to_string(&key_path).map_err(|e| format!("Read key: {e}"))?;
    let key_file: KeyFile =
        serde_json::from_str(&content).map_err(|e| format!("Parse key: {e}"))?;
    let key = EncryptionKey::derive(&password, &key_file.salt)?;
    key.encrypt_string(&plaintext)
}

/// Decrypt a string value using the password-derived key.
#[tauri::command]
pub async fn ai_decrypt(password: String, encrypted: String) -> Result<String, String> {
    let key_path = get_key_file_path()?;
    let content = std::fs::read_to_string(&key_path).map_err(|e| format!("Read key: {e}"))?;
    let key_file: KeyFile =
        serde_json::from_str(&content).map_err(|e| format!("Parse key: {e}"))?;
    let key = EncryptionKey::derive(&password, &key_file.salt)?;
    key.decrypt_string(&encrypted)
}
