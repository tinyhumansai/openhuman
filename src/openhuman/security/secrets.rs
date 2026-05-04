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
#[path = "secrets_tests.rs"]
mod tests;
