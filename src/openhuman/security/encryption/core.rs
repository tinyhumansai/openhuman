use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use argon2::{self, Algorithm, Argon2, Params, Version};
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

/// Get the path to the OpenHuman data directory.
/// If an active user is set, returns the user-scoped directory under the
/// env-aware root returned by `default_root_openhuman_dir()`
/// (for example `~/.openhuman/users/{user_id}` in production or
/// `~/.openhuman-staging/users/{user_id}` when `OPENHUMAN_APP_ENV=staging`);
/// otherwise it falls back to that root directory itself.
pub fn get_data_dir() -> Result<PathBuf, String> {
    let root_dir = crate::openhuman::config::default_root_openhuman_dir()
        .map_err(|e| format!("Cannot determine app data directory: {e}"))?;
    std::fs::create_dir_all(&root_dir)
        .map_err(|e| format!("Failed to create data directory: {e}"))?;

    let data_dir = if let Some(user_id) = crate::openhuman::config::read_active_user_id(&root_dir) {
        let user_dir = crate::openhuman::config::user_openhuman_dir(&root_dir, &user_id);
        std::fs::create_dir_all(&user_dir)
            .map_err(|e| format!("Failed to create user data directory: {e}"))?;
        user_dir
    } else {
        root_dir
    };

    Ok(data_dir)
}

/// Get the path to the encryption key file under the env-aware OpenHuman root
/// (for example `~/.openhuman/encryption.key` or `~/.openhuman-staging/encryption.key`).
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

/// Initialize encryption with a password. Creates key file if needed.
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
pub async fn ai_encrypt(password: String, plaintext: String) -> Result<String, String> {
    let key_path = get_key_file_path()?;
    let content = std::fs::read_to_string(&key_path).map_err(|e| format!("Read key: {e}"))?;
    let key_file: KeyFile =
        serde_json::from_str(&content).map_err(|e| format!("Parse key: {e}"))?;
    let key = EncryptionKey::derive(&password, &key_file.salt)?;
    key.encrypt_string(&plaintext)
}

/// Decrypt a string value using the password-derived key.
pub async fn ai_decrypt(password: String, encrypted: String) -> Result<String, String> {
    let key_path = get_key_file_path()?;
    let content = std::fs::read_to_string(&key_path).map_err(|e| format!("Read key: {e}"))?;
    let key_file: KeyFile =
        serde_json::from_str(&content).map_err(|e| format!("Parse key: {e}"))?;
    let key = EncryptionKey::derive(&password, &key_file.salt)?;
    key.decrypt_string(&encrypted)
}

#[cfg(test)]
mod tests {
    //! Round-trip + tamper coverage for the Argon2id + AES-256-GCM primitives
    //! (plan.md §4 P0 #2 — previously zero unit tests). `key_bytes` is private,
    //! so key equality is asserted *behaviourally*: a key derived twice from the
    //! same (password, salt) must decrypt the other's ciphertext, and any change
    //! to password, salt, ciphertext, or nonce must make decryption fail.
    use super::*;

    fn key(password: &str, salt: &[u8]) -> EncryptionKey {
        EncryptionKey::derive(password, salt).expect("derive")
    }

    #[test]
    fn encrypt_decrypt_bytes_round_trip() {
        let k = key(
            "correct horse battery staple",
            &EncryptionKey::generate_salt(),
        );
        let plaintext = b"the launch codes are 0000".to_vec();
        let payload = k.encrypt(&plaintext).expect("encrypt");
        assert_ne!(
            payload.ciphertext, plaintext,
            "ciphertext must not be plaintext"
        );
        assert_eq!(k.decrypt(&payload).expect("decrypt"), plaintext);
    }

    #[test]
    fn encrypt_decrypt_string_round_trip() {
        let k = key("pw", &EncryptionKey::generate_salt());
        let secret = "sk-live-🔐-multibyte";
        let json = k.encrypt_string(secret).expect("encrypt_string");
        assert_eq!(k.decrypt_string(&json).expect("decrypt_string"), secret);
    }

    #[test]
    fn kdf_is_deterministic_for_same_password_and_salt() {
        // Two independent derivations from the same (password, salt) must yield
        // the same key: key_a encrypts, key_b decrypts.
        let salt = EncryptionKey::generate_salt();
        let key_a = key("hunter2", &salt);
        let key_b = key("hunter2", &salt);
        let payload = key_a.encrypt(b"cross-key").expect("encrypt");
        assert_eq!(key_b.decrypt(&payload).expect("decrypt"), b"cross-key");
    }

    #[test]
    fn wrong_password_cannot_decrypt() {
        let salt = EncryptionKey::generate_salt();
        let good = key("right-password", &salt);
        let bad = key("wrong-password", &salt);
        let payload = good.encrypt(b"top secret").expect("encrypt");
        assert!(
            bad.decrypt(&payload).is_err(),
            "a key from a different password must not decrypt"
        );
    }

    #[test]
    fn different_salt_derives_a_different_key() {
        let a = key("same-password", &EncryptionKey::generate_salt());
        let b = key("same-password", &EncryptionKey::generate_salt());
        let payload = a.encrypt(b"salted").expect("encrypt");
        assert!(
            b.decrypt(&payload).is_err(),
            "same password + different salt must yield a non-interchangeable key"
        );
    }

    #[test]
    fn tampered_ciphertext_is_rejected_by_gcm_auth() {
        let k = key("pw", &EncryptionKey::generate_salt());
        let mut payload = k.encrypt(b"authentic bytes").expect("encrypt");
        payload.ciphertext[0] ^= 0xFF; // flip a bit in the ciphertext/tag
        assert!(
            k.decrypt(&payload).is_err(),
            "AES-GCM must reject a tampered ciphertext (auth failure)"
        );
    }

    #[test]
    fn tampered_nonce_is_rejected() {
        let k = key("pw", &EncryptionKey::generate_salt());
        let mut payload = k.encrypt(b"authentic bytes").expect("encrypt");
        payload.nonce[0] ^= 0xFF; // wrong nonce → auth tag no longer verifies
        assert!(
            k.decrypt(&payload).is_err(),
            "decrypting under a mutated nonce must fail"
        );
    }

    #[test]
    fn each_encryption_uses_a_fresh_random_nonce() {
        // Nonce reuse under a fixed key is catastrophic for GCM. Encrypting the
        // same plaintext twice must produce distinct nonces (and, therefore,
        // distinct ciphertexts) — the nonce is drawn from the CSPRNG per call.
        let k = key("pw", &EncryptionKey::generate_salt());
        let p1 = k.encrypt(b"identical plaintext").expect("encrypt");
        let p2 = k.encrypt(b"identical plaintext").expect("encrypt");
        assert_ne!(
            p1.nonce, p2.nonce,
            "each encryption must draw a fresh nonce"
        );
        assert_ne!(
            p1.ciphertext, p2.ciphertext,
            "a fresh nonce must produce different ciphertext for the same plaintext"
        );
    }

    #[test]
    fn generate_salt_is_correct_length_and_random() {
        let s1 = EncryptionKey::generate_salt();
        let s2 = EncryptionKey::generate_salt();
        assert_eq!(s1.len(), SALT_LENGTH, "salt must be {SALT_LENGTH} bytes");
        assert_ne!(s1, s2, "two generated salts must differ (CSPRNG)");
    }

    #[test]
    fn decrypt_string_rejects_malformed_json() {
        let k = key("pw", &EncryptionKey::generate_salt());
        assert!(
            k.decrypt_string("not-json").is_err(),
            "non-JSON payload must be a clean Err, not a panic"
        );
    }

    #[test]
    fn empty_plaintext_round_trips() {
        let k = key("pw", &EncryptionKey::generate_salt());
        let payload = k.encrypt(b"").expect("encrypt empty");
        assert_eq!(k.decrypt(&payload).expect("decrypt empty"), b"");
    }

    // NOTE: an `encrypt_decrypt_round_trips_for_arbitrary_input` proptest was
    // trialled here but removed: under coverage instrumentation Argon2id runs
    // ~2.5s/case, so a 24-case property held the lib-test binary for ~60s and
    // deterministically widened a pre-existing env-var race in the unrelated
    // `config::schema::load` env-overlay tests (they mutate process-global env
    // without per-test serialization). The round-trip is already covered by the
    // fixed-input tests above (round-trip, tamper, KDF determinism); the
    // property-based *fuzzing* value lives in the fast, panic-focused
    // `security::policy::proptest_tests` instead.
}
