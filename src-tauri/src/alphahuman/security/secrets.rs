// Encrypted secret store — defense-in-depth for API keys and tokens.
//
// Secrets are encrypted using ChaCha20-Poly1305 AEAD with a random key stored
// in `{data_dir}/openhuman/.secret_key` with restrictive file permissions (0600). The
// config file stores only hex-encoded ciphertext, never plaintext keys.
//
// Each encryption generates a fresh random 12-byte nonce, prepended to the
// ciphertext. The Poly1305 authentication tag prevents tampering.
//
// This prevents:
//   - Plaintext exposure in config files
//   - Casual `grep` or `git log` leaks
//   - Accidental commit of raw API keys
//   - Known-plaintext attacks (unlike the previous XOR cipher)
//   - Ciphertext tampering (authenticated encryption)
//
// For sovereign users who prefer plaintext, `secrets.encrypt = false` disables this.
//
// Migration: values with the legacy `enc:` prefix (XOR cipher) are decrypted
// using the old algorithm for backward compatibility. New encryptions always
// produce `enc2:` (ChaCha20-Poly1305).

use anyhow::{Context, Result};
use chacha20poly1305::aead::{Aead, KeyInit, OsRng};
use chacha20poly1305::{AeadCore, ChaCha20Poly1305, Key, Nonce};
use std::fs;
use std::path::{Path, PathBuf};

/// Length of the random encryption key in bytes (256-bit, matches `ChaCha20`).
const KEY_LEN: usize = 32;

/// ChaCha20-Poly1305 nonce length in bytes.
const NONCE_LEN: usize = 12;

/// Manages encrypted storage of secrets (API keys, tokens, etc.)
#[derive(Debug, Clone)]
pub struct SecretStore {
    /// Path to the key file (`{data_dir}/openhuman/.secret_key`)
    key_path: PathBuf,
    /// Whether encryption is enabled
    enabled: bool,
}

impl SecretStore {
    /// Create a new secret store rooted at the given directory.
    pub fn new(openhuman_dir: &Path, enabled: bool) -> Self {
        Self {
            key_path: openhuman_dir.join(".secret_key"),
            enabled,
        }
    }

    /// Encrypt a plaintext secret. Returns hex-encoded ciphertext prefixed with `enc2:`.
    /// Format: `enc2:<hex(nonce ‖ ciphertext ‖ tag)>` (12 + N + 16 bytes).
    /// If encryption is disabled, returns the plaintext as-is.
    pub fn encrypt(&self, plaintext: &str) -> Result<String> {
        if !self.enabled || plaintext.is_empty() {
            return Ok(plaintext.to_string());
        }

        let key_bytes = self.load_or_create_key()?;
        let key = Key::from_slice(&key_bytes);
        let cipher = ChaCha20Poly1305::new(key);

        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ciphertext = cipher
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|e| anyhow::anyhow!("Encryption failed: {e}"))?;

        // Prepend nonce to ciphertext for storage
        let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ciphertext);

        Ok(format!("enc2:{}", hex_encode(&blob)))
    }

    /// Decrypt a secret.
    /// - `enc2:` prefix → ChaCha20-Poly1305 (current format)
    /// - `enc:` prefix → legacy XOR cipher (backward compatibility for migration)
    /// - No prefix → returned as-is (plaintext config)
    ///
    /// **Warning**: Legacy `enc:` values are insecure. Use `decrypt_and_migrate` to
    /// automatically upgrade them to the secure `enc2:` format.
    pub fn decrypt(&self, value: &str) -> Result<String> {
        if let Some(hex_str) = value.strip_prefix("enc2:") {
            self.decrypt_chacha20(hex_str)
        } else if let Some(hex_str) = value.strip_prefix("enc:") {
            self.decrypt_legacy_xor(hex_str)
        } else {
            Ok(value.to_string())
        }
    }

    /// Decrypt a secret and return a migrated `enc2:` value if the input used legacy `enc:` format.
    ///
    /// Returns `(plaintext, Some(new_enc2_value))` if migration occurred, or
    /// `(plaintext, None)` if no migration was needed.
    ///
    /// This allows callers to persist the upgraded value back to config.
    pub fn decrypt_and_migrate(&self, value: &str) -> Result<(String, Option<String>)> {
        if let Some(hex_str) = value.strip_prefix("enc2:") {
            // Already using secure format — no migration needed
            let plaintext = self.decrypt_chacha20(hex_str)?;
            Ok((plaintext, None))
        } else if let Some(hex_str) = value.strip_prefix("enc:") {
            // Legacy XOR cipher — decrypt and re-encrypt with ChaCha20-Poly1305
            log::warn!(
                "Decrypting legacy XOR-encrypted secret (enc: prefix). \
                 This format is insecure and will be removed in a future release. \
                 The secret will be automatically migrated to enc2: (ChaCha20-Poly1305)."
            );
            let plaintext = self.decrypt_legacy_xor(hex_str)?;
            let migrated = self.encrypt(&plaintext)?;
            Ok((plaintext, Some(migrated)))
        } else {
            // Plaintext — no migration needed
            Ok((value.to_string(), None))
        }
    }

    /// Check if a value uses the legacy `enc:` format that should be migrated.
    pub fn needs_migration(value: &str) -> bool {
        value.starts_with("enc:")
    }

    /// Decrypt using ChaCha20-Poly1305 (current secure format).
    fn decrypt_chacha20(&self, hex_str: &str) -> Result<String> {
        let blob =
            hex_decode(hex_str).context("Failed to decode encrypted secret (corrupt hex)")?;
        anyhow::ensure!(
            blob.len() > NONCE_LEN,
            "Encrypted value too short (missing nonce)"
        );

        let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
        let nonce = Nonce::from_slice(nonce_bytes);
        let key_bytes = self.load_or_create_key()?;
        let key = Key::from_slice(&key_bytes);
        let cipher = ChaCha20Poly1305::new(key);

        let plaintext_bytes = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| anyhow::anyhow!("Decryption failed — wrong key or tampered data"))?;

        String::from_utf8(plaintext_bytes)
            .context("Decrypted secret is not valid UTF-8 — corrupt data")
    }

    /// Decrypt using legacy XOR cipher (insecure, for backward compatibility only).
    fn decrypt_legacy_xor(&self, hex_str: &str) -> Result<String> {
        let ciphertext = hex_decode(hex_str)
            .context("Failed to decode legacy encrypted secret (corrupt hex)")?;
        let key = self.load_or_create_key()?;
        let plaintext_bytes = xor_cipher(&ciphertext, &key);
        String::from_utf8(plaintext_bytes)
            .context("Decrypted legacy secret is not valid UTF-8 — wrong key or corrupt data")
    }

    /// Check if a value is already encrypted (current or legacy format).
    pub fn is_encrypted(value: &str) -> bool {
        value.starts_with("enc2:") || value.starts_with("enc:")
    }

    /// Check if a value uses the secure `enc2:` format.
    pub fn is_secure_encrypted(value: &str) -> bool {
        value.starts_with("enc2:")
    }

    /// Load the encryption key from disk, or create one if it doesn't exist.
    fn load_or_create_key(&self) -> Result<Vec<u8>> {
        if self.key_path.exists() {
            let hex_key =
                fs::read_to_string(&self.key_path).context("Failed to read secret key file")?;
            hex_decode(hex_key.trim()).context("Secret key file is corrupt")
        } else {
            let key = generate_random_key();
            if let Some(parent) = self.key_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&self.key_path, hex_encode(&key))
                .context("Failed to write secret key file")?;

            // Set restrictive permissions
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&self.key_path, fs::Permissions::from_mode(0o600))
                    .context("Failed to set key file permissions")?;
            }
            #[cfg(windows)]
            {
                // On Windows, use icacls to restrict permissions to current user only
                let username = std::env::var("USERNAME").unwrap_or_default();
                let Some(grant_arg) = build_windows_icacls_grant_arg(&username) else {
                    log::warn!(
                        "USERNAME environment variable is empty; \
                         cannot restrict key file permissions via icacls"
                    );
                    return Ok(key);
                };

                match std::process::Command::new("icacls")
                    .arg(&self.key_path)
                    .args(["/inheritance:r", "/grant:r"])
                    .arg(grant_arg)
                    .output()
                {
                    Ok(o) if !o.status.success() => {
                        log::warn!(
                            "Failed to set key file permissions via icacls (exit code {:?})",
                            o.status.code()
                        );
                    }
                    Err(e) => {
                        log::warn!("Could not set key file permissions: {e}");
                    }
                    _ => {
                        log::debug!("Key file permissions restricted via icacls");
                    }
                }
            }

            Ok(key)
        }
    }
}

/// XOR cipher with repeating key. Same function for encrypt and decrypt.
fn xor_cipher(data: &[u8], key: &[u8]) -> Vec<u8> {
    if key.is_empty() {
        return data.to_vec();
    }
    data.iter()
        .enumerate()
        .map(|(i, &b)| b ^ key[i % key.len()])
        .collect()
}

/// Generate a random 256-bit key using the OS CSPRNG.
///
/// Uses `OsRng` (via `getrandom`) directly, providing full 256-bit entropy
/// without the fixed version/variant bits that UUID v4 introduces.
fn generate_random_key() -> Vec<u8> {
    ChaCha20Poly1305::generate_key(&mut OsRng).to_vec()
}

/// Hex-encode bytes to a lowercase hex string.
fn hex_encode(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 2);
    for b in data {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Build the `/grant` argument for `icacls` using a normalized username.
/// Returns `None` when the username is empty or whitespace-only.
fn build_windows_icacls_grant_arg(username: &str) -> Option<String> {
    let normalized = username.trim();
    if normalized.is_empty() {
        return None;
    }
    Some(format!("{normalized}:F"))
}

/// Hex-decode a hex string to bytes.
#[allow(clippy::manual_is_multiple_of)]
fn hex_decode(hex: &str) -> Result<Vec<u8>> {
    if (hex.len() & 1) != 0 {
        anyhow::bail!("Hex string has odd length");
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .map_err(|e| anyhow::anyhow!("Invalid hex at position {i}: {e}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── SecretStore basics ─────────────────────────────────────

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let store = SecretStore::new(tmp.path(), true);
        let secret = "sk-my-secret-api-key-12345";

        let encrypted = store.encrypt(secret).unwrap();
        assert!(encrypted.starts_with("enc2:"), "Should have enc2: prefix");
        assert_ne!(encrypted, secret, "Should not be plaintext");

        let decrypted = store.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, secret, "Roundtrip must preserve original");
    }

    #[test]
    fn encrypt_empty_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let store = SecretStore::new(tmp.path(), true);
        let result = store.encrypt("").unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn decrypt_plaintext_passthrough() {
        let tmp = TempDir::new().unwrap();
        let store = SecretStore::new(tmp.path(), true);
        // Values without "enc:"/"enc2:" prefix are returned as-is (backward compat)
        let result = store.decrypt("sk-plaintext-key").unwrap();
        assert_eq!(result, "sk-plaintext-key");
    }

    #[test]
    fn disabled_store_returns_plaintext() {
        let tmp = TempDir::new().unwrap();
        let store = SecretStore::new(tmp.path(), false);
        let result = store.encrypt("sk-secret").unwrap();
        assert_eq!(result, "sk-secret", "Disabled store should not encrypt");
    }

    #[test]
    fn is_encrypted_detects_prefix() {
        assert!(SecretStore::is_encrypted("enc2:aabbcc"));
        assert!(SecretStore::is_encrypted("enc:aabbcc")); // legacy
        assert!(!SecretStore::is_encrypted("sk-plaintext"));
        assert!(!SecretStore::is_encrypted(""));
    }

    #[tokio::test]
    async fn key_file_created_on_first_encrypt() {
        let tmp = TempDir::new().unwrap();
        let store = SecretStore::new(tmp.path(), true);
        assert!(!store.key_path.exists());

        store.encrypt("test").unwrap();
        assert!(store.key_path.exists(), "Key file should be created");

        let key_hex = tokio::fs::read_to_string(&store.key_path).await.unwrap();
        assert_eq!(
            key_hex.len(),
            KEY_LEN * 2,
            "Key should be {KEY_LEN} bytes hex-encoded"
        );
    }

    #[test]
    fn encrypting_same_value_produces_different_ciphertext() {
        let tmp = TempDir::new().unwrap();
        let store = SecretStore::new(tmp.path(), true);

        let e1 = store.encrypt("secret").unwrap();
        let e2 = store.encrypt("secret").unwrap();
        assert_ne!(
            e1, e2,
            "AEAD with random nonce should produce different ciphertext each time"
        );

        // Both should still decrypt to the same value
        assert_eq!(store.decrypt(&e1).unwrap(), "secret");
        assert_eq!(store.decrypt(&e2).unwrap(), "secret");
    }

    #[test]
    fn different_stores_same_dir_interop() {
        let tmp = TempDir::new().unwrap();
        let store1 = SecretStore::new(tmp.path(), true);
        let store2 = SecretStore::new(tmp.path(), true);

        let encrypted = store1.encrypt("cross-store-secret").unwrap();
        let decrypted = store2.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, "cross-store-secret");
    }

    #[test]
    fn unicode_secret_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let store = SecretStore::new(tmp.path(), true);
        let secret = "sk-日本語テスト-émojis-🦀";

        let encrypted = store.encrypt(secret).unwrap();
        let decrypted = store.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, secret);
    }

    #[test]
    fn long_secret_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let store = SecretStore::new(tmp.path(), true);
        let secret = "a".repeat(10_000);

        let encrypted = store.encrypt(&secret).unwrap();
        let decrypted = store.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, secret);
    }

    #[test]
    fn corrupt_hex_returns_error() {
        let tmp = TempDir::new().unwrap();
        let store = SecretStore::new(tmp.path(), true);
        let result = store.decrypt("enc2:not-valid-hex!!");
        assert!(result.is_err());
    }

    #[test]
    fn tampered_ciphertext_detected() {
        let tmp = TempDir::new().unwrap();
        let store = SecretStore::new(tmp.path(), true);
        let encrypted = store.encrypt("sensitive-data").unwrap();

        // Flip a bit in the ciphertext (after the "enc2:" prefix)
        let hex_str = &encrypted[5..];
        let mut blob = hex_decode(hex_str).unwrap();
        // Modify a byte in the ciphertext portion (after the 12-byte nonce)
        if blob.len() > NONCE_LEN {
            blob[NONCE_LEN] ^= 0xff;
        }
        let tampered = format!("enc2:{}", hex_encode(&blob));

        let result = store.decrypt(&tampered);
        assert!(result.is_err(), "Tampered ciphertext must be rejected");
    }

    #[test]
    fn wrong_key_detected() {
        let tmp1 = TempDir::new().unwrap();
        let tmp2 = TempDir::new().unwrap();
        let store1 = SecretStore::new(tmp1.path(), true);
        let store2 = SecretStore::new(tmp2.path(), true);

        let encrypted = store1.encrypt("secret-for-store1").unwrap();
        let result = store2.decrypt(&encrypted);
        assert!(result.is_err(), "Decrypting with a different key must fail");
    }

    #[test]
    fn truncated_ciphertext_returns_error() {
        let tmp = TempDir::new().unwrap();
        let store = SecretStore::new(tmp.path(), true);
        // Only a few bytes — shorter than nonce
        let result = store.decrypt("enc2:aabbccdd");
        assert!(result.is_err(), "Too-short ciphertext must be rejected");
    }

    // ── Legacy XOR backward compatibility ───────────────────────

    #[test]
    fn legacy_xor_decrypt_still_works() {
        let tmp = TempDir::new().unwrap();
        let store = SecretStore::new(tmp.path(), true);

        // Trigger key creation via an encrypt call
        let _ = store.encrypt("setup").unwrap();
        let key = store.load_or_create_key().unwrap();

        // Manually produce a legacy XOR-encrypted value
        let plaintext = "sk-legacy-api-key";
        let ciphertext = xor_cipher(plaintext.as_bytes(), &key);
        let legacy_value = format!("enc:{}", hex_encode(&ciphertext));

        // Store should still be able to decrypt legacy values
        let decrypted = store.decrypt(&legacy_value).unwrap();
        assert_eq!(decrypted, plaintext, "Legacy XOR values must still decrypt");
    }

    // ── Migration tests ─────────────────────────────────────────

    #[test]
    fn needs_migration_detects_legacy_prefix() {
        assert!(SecretStore::needs_migration("enc:aabbcc"));
        assert!(!SecretStore::needs_migration("enc2:aabbcc"));
        assert!(!SecretStore::needs_migration("sk-plaintext"));
        assert!(!SecretStore::needs_migration(""));
    }

    #[test]
    fn is_secure_encrypted_detects_enc2_only() {
        assert!(SecretStore::is_secure_encrypted("enc2:aabbcc"));
        assert!(!SecretStore::is_secure_encrypted("enc:aabbcc"));
        assert!(!SecretStore::is_secure_encrypted("sk-plaintext"));
        assert!(!SecretStore::is_secure_encrypted(""));
    }

    #[test]
    fn decrypt_and_migrate_returns_none_for_enc2() {
        let tmp = TempDir::new().unwrap();
        let store = SecretStore::new(tmp.path(), true);

        let encrypted = store.encrypt("my-secret").unwrap();
        assert!(encrypted.starts_with("enc2:"));

        let (plaintext, migrated) = store.decrypt_and_migrate(&encrypted).unwrap();
        assert_eq!(plaintext, "my-secret");
        assert!(
            migrated.is_none(),
            "enc2: values should not trigger migration"
        );
    }

    #[test]
    fn decrypt_and_migrate_returns_none_for_plaintext() {
        let tmp = TempDir::new().unwrap();
        let store = SecretStore::new(tmp.path(), true);

        let (plaintext, migrated) = store.decrypt_and_migrate("sk-plaintext-key").unwrap();
        assert_eq!(plaintext, "sk-plaintext-key");
        assert!(
            migrated.is_none(),
            "Plaintext values should not trigger migration"
        );
    }

    #[test]
    fn decrypt_and_migrate_upgrades_legacy_xor() {
        let tmp = TempDir::new().unwrap();
        let store = SecretStore::new(tmp.path(), true);

        // Create key first
        let _ = store.encrypt("setup").unwrap();
        let key = store.load_or_create_key().unwrap();

        // Manually create a legacy XOR-encrypted value
        let plaintext = "sk-legacy-secret-to-migrate";
        let ciphertext = xor_cipher(plaintext.as_bytes(), &key);
        let legacy_value = format!("enc:{}", hex_encode(&ciphertext));

        // Verify it needs migration
        assert!(SecretStore::needs_migration(&legacy_value));

        // Decrypt and migrate
        let (decrypted, migrated) = store.decrypt_and_migrate(&legacy_value).unwrap();
        assert_eq!(decrypted, plaintext, "Plaintext must match original");
        assert!(migrated.is_some(), "Legacy value should trigger migration");

        let new_value = migrated.unwrap();
        assert!(
            new_value.starts_with("enc2:"),
            "Migrated value must use enc2: prefix"
        );
        assert!(
            !SecretStore::needs_migration(&new_value),
            "Migrated value should not need migration"
        );

        // Verify the migrated value decrypts correctly
        let (decrypted2, migrated2) = store.decrypt_and_migrate(&new_value).unwrap();
        assert_eq!(
            decrypted2, plaintext,
            "Migrated value must decrypt to same plaintext"
        );
        assert!(
            migrated2.is_none(),
            "Migrated value should not trigger another migration"
        );
    }

    #[test]
    fn decrypt_and_migrate_handles_unicode() {
        let tmp = TempDir::new().unwrap();
        let store = SecretStore::new(tmp.path(), true);

        let _ = store.encrypt("setup").unwrap();
        let key = store.load_or_create_key().unwrap();

        let plaintext = "sk-日本語-émojis-🦀-тест";
        let ciphertext = xor_cipher(plaintext.as_bytes(), &key);
        let legacy_value = format!("enc:{}", hex_encode(&ciphertext));

        let (decrypted, migrated) = store.decrypt_and_migrate(&legacy_value).unwrap();
        assert_eq!(decrypted, plaintext);
        assert!(migrated.is_some());

        // Verify migrated value works
        let new_value = migrated.unwrap();
        let (decrypted2, _) = store.decrypt_and_migrate(&new_value).unwrap();
        assert_eq!(decrypted2, plaintext);
    }

    #[test]
    fn decrypt_and_migrate_handles_empty_secret() {
        let tmp = TempDir::new().unwrap();
        let store = SecretStore::new(tmp.path(), true);

        let _ = store.encrypt("setup").unwrap();
        let key = store.load_or_create_key().unwrap();

        // Empty plaintext XOR-encrypted
        let plaintext = "";
        let ciphertext = xor_cipher(plaintext.as_bytes(), &key);
        let legacy_value = format!("enc:{}", hex_encode(&ciphertext));

        let (decrypted, migrated) = store.decrypt_and_migrate(&legacy_value).unwrap();
        assert_eq!(decrypted, plaintext);
        // Empty string encryption returns empty string (not enc2:)
        assert!(migrated.is_some());
        assert_eq!(migrated.unwrap(), "");
    }

    #[test]
    fn decrypt_and_migrate_handles_long_secret() {
        let tmp = TempDir::new().unwrap();
        let store = SecretStore::new(tmp.path(), true);

        let _ = store.encrypt("setup").unwrap();
        let key = store.load_or_create_key().unwrap();

        let plaintext = "a".repeat(10_000);
        let ciphertext = xor_cipher(plaintext.as_bytes(), &key);
        let legacy_value = format!("enc:{}", hex_encode(&ciphertext));

        let (decrypted, migrated) = store.decrypt_and_migrate(&legacy_value).unwrap();
        assert_eq!(decrypted, plaintext);
        assert!(migrated.is_some());

        let new_value = migrated.unwrap();
        let (decrypted2, _) = store.decrypt_and_migrate(&new_value).unwrap();
        assert_eq!(decrypted2, plaintext);
    }

    #[test]
    fn decrypt_and_migrate_fails_on_corrupt_legacy_hex() {
        let tmp = TempDir::new().unwrap();
        let store = SecretStore::new(tmp.path(), true);
        let _ = store.encrypt("setup").unwrap();

        let result = store.decrypt_and_migrate("enc:not-valid-hex!!");
        assert!(result.is_err(), "Corrupt hex should fail");
    }

    #[test]
    fn decrypt_and_migrate_wrong_key_produces_garbage_or_fails() {
        let tmp1 = TempDir::new().unwrap();
        let tmp2 = TempDir::new().unwrap();
        let store1 = SecretStore::new(tmp1.path(), true);
        let store2 = SecretStore::new(tmp2.path(), true);

        // Create keys for both stores
        let _ = store1.encrypt("setup").unwrap();
        let _ = store2.encrypt("setup").unwrap();
        let key1 = store1.load_or_create_key().unwrap();

        // Encrypt with store1's key
        let plaintext = "secret-for-store1";
        let ciphertext = xor_cipher(plaintext.as_bytes(), &key1);
        let legacy_value = format!("enc:{}", hex_encode(&ciphertext));

        // Decrypt with store2 — XOR will produce garbage bytes
        // This may fail with UTF-8 error or succeed with garbage plaintext
        match store2.decrypt_and_migrate(&legacy_value) {
            Ok((decrypted, _)) => {
                // If it succeeds, the plaintext should be garbage (not the original)
                assert_ne!(
                    decrypted, plaintext,
                    "Wrong key should produce garbage plaintext"
                );
            }
            Err(e) => {
                // Expected: UTF-8 decoding failure from garbage bytes
                assert!(
                    e.to_string().contains("UTF-8"),
                    "Error should be UTF-8 related: {e}"
                );
            }
        }
    }

    #[test]
    fn migration_produces_different_ciphertext_each_time() {
        let tmp = TempDir::new().unwrap();
        let store = SecretStore::new(tmp.path(), true);

        let _ = store.encrypt("setup").unwrap();
        let key = store.load_or_create_key().unwrap();

        let plaintext = "sk-same-secret";
        let ciphertext = xor_cipher(plaintext.as_bytes(), &key);
        let legacy_value = format!("enc:{}", hex_encode(&ciphertext));

        let (_, migrated1) = store.decrypt_and_migrate(&legacy_value).unwrap();
        let (_, migrated2) = store.decrypt_and_migrate(&legacy_value).unwrap();

        assert!(migrated1.is_some());
        assert!(migrated2.is_some());
        assert_ne!(
            migrated1.unwrap(),
            migrated2.unwrap(),
            "Each migration should produce different ciphertext (random nonce)"
        );
    }

    #[test]
    fn migrated_value_is_tamper_resistant() {
        let tmp = TempDir::new().unwrap();
        let store = SecretStore::new(tmp.path(), true);

        let _ = store.encrypt("setup").unwrap();
        let key = store.load_or_create_key().unwrap();

        let plaintext = "sk-sensitive-data";
        let ciphertext = xor_cipher(plaintext.as_bytes(), &key);
        let legacy_value = format!("enc:{}", hex_encode(&ciphertext));

        let (_, migrated) = store.decrypt_and_migrate(&legacy_value).unwrap();
        let new_value = migrated.unwrap();

        // Tamper with the migrated value
        let hex_str = &new_value[5..];
        let mut blob = hex_decode(hex_str).unwrap();
        if blob.len() > NONCE_LEN {
            blob[NONCE_LEN] ^= 0xff;
        }
        let tampered = format!("enc2:{}", hex_encode(&blob));

        let result = store.decrypt_and_migrate(&tampered);
        assert!(result.is_err(), "Tampered migrated value must be rejected");
    }

    // ── Low-level helpers ───────────────────────────────────────

    #[test]
    fn xor_cipher_roundtrip() {
        let key = b"testkey123";
        let data = b"hello world";
        let encrypted = xor_cipher(data, key);
        let decrypted = xor_cipher(&encrypted, key);
        assert_eq!(decrypted, data);
    }

    #[test]
    fn xor_cipher_empty_key() {
        let data = b"passthrough";
        let result = xor_cipher(data, &[]);
        assert_eq!(result, data);
    }

    #[test]
    fn hex_roundtrip() {
        let data = vec![0x00, 0x01, 0xfe, 0xff, 0xab, 0xcd];
        let encoded = hex_encode(&data);
        assert_eq!(encoded, "0001feffabcd");
        let decoded = hex_decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn hex_decode_odd_length_fails() {
        assert!(hex_decode("abc").is_err());
    }

    #[test]
    fn hex_decode_invalid_chars_fails() {
        assert!(hex_decode("zzzz").is_err());
    }

    #[test]
    fn windows_icacls_grant_arg_rejects_empty_username() {
        assert_eq!(build_windows_icacls_grant_arg(""), None);
        assert_eq!(build_windows_icacls_grant_arg("   \t\n"), None);
    }

    #[test]
    fn windows_icacls_grant_arg_trims_username() {
        assert_eq!(
            build_windows_icacls_grant_arg("  alice  "),
            Some("alice:F".to_string())
        );
    }

    #[test]
    fn windows_icacls_grant_arg_preserves_valid_characters() {
        assert_eq!(
            build_windows_icacls_grant_arg("DOMAIN\\svc-user"),
            Some("DOMAIN\\svc-user:F".to_string())
        );
    }

    #[test]
    fn generate_random_key_correct_length() {
        let key = generate_random_key();
        assert_eq!(key.len(), KEY_LEN);
    }

    #[test]
    fn generate_random_key_not_all_zeros() {
        let key = generate_random_key();
        assert!(key.iter().any(|&b| b != 0), "Key should not be all zeros");
    }

    #[test]
    fn two_random_keys_differ() {
        let k1 = generate_random_key();
        let k2 = generate_random_key();
        assert_ne!(k1, k2, "Two random keys should differ");
    }

    #[test]
    fn generate_random_key_has_no_uuid_fixed_bits() {
        // UUID v4 has fixed bits at positions 6 (version = 0b0100xxxx) and
        // 8 (variant = 0b10xxxxxx). A direct CSPRNG key should not consistently
        // have these patterns across multiple samples.
        let mut version_match = 0;
        let mut variant_match = 0;
        let samples = 100;
        for _ in 0..samples {
            let key = generate_random_key();
            // In UUID v4, byte 6 always has top nibble = 0x4
            if key[6] & 0xf0 == 0x40 {
                version_match += 1;
            }
            // In UUID v4, byte 8 always has top 2 bits = 0b10
            if key[8] & 0xc0 == 0x80 {
                variant_match += 1;
            }
        }
        // With true randomness, each pattern should appear ~1/16 and ~1/4 of
        // the time. UUID would hit 100/100 on both. Allow generous margin.
        assert!(
            version_match < 30,
            "byte[6] matched UUID v4 version nibble {version_match}/100 times — \
             likely still using UUID-based key generation"
        );
        assert!(
            variant_match < 50,
            "byte[8] matched UUID v4 variant bits {variant_match}/100 times — \
             likely still using UUID-based key generation"
        );
    }

    #[cfg(unix)]
    #[test]
    fn key_file_has_restricted_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let store = SecretStore::new(tmp.path(), true);
        store.encrypt("trigger key creation").unwrap();

        let perms = fs::metadata(&store.key_path).unwrap().permissions();
        assert_eq!(
            perms.mode() & 0o777,
            0o600,
            "Key file must be owner-only (0600)"
        );
    }
}
