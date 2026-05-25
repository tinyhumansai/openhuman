// Encrypted secret store — defense-in-depth for API keys and tokens.
//
// Secrets are encrypted using ChaCha20-Poly1305 AEAD with a random key stored
// in the OS keychain for normal app environments. Legacy installs may still
// have `{data_dir}/openhuman/.secret_key`; those keys are migrated into the
// keychain on first use. Unit tests retain the file path for deterministic
// isolation. The config file stores only hex-encoded ciphertext, never
// plaintext keys.
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
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Length of the random encryption key in bytes (256-bit, matches `ChaCha20`).
const KEY_LEN: usize = 32;

/// ChaCha20-Poly1305 nonce length in bytes.
const NONCE_LEN: usize = 12;

/// Keychain slot used for the SecretStore master key.
const KEYCHAIN_MASTER_KEY: &str = "secretstore.master_key";

/// Manages encrypted storage of secrets (API keys, tokens, etc.)
#[derive(Debug, Clone)]
pub struct SecretStore {
    /// Legacy path to the on-disk key file (`{data_dir}/openhuman/.secret_key`).
    /// Still used for unit tests and one-time migration into the keychain.
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

    fn use_legacy_file_key(&self) -> bool {
        cfg!(test)
    }

    fn keyring_user_id(&self) -> String {
        keyring_user_id_from_dir(self.key_path.parent().unwrap_or_else(|| Path::new(".")))
    }

    fn load_or_create_key_from_keyring(&self) -> Result<Vec<u8>> {
        let user_id = self.keyring_user_id();
        match crate::openhuman::keyring::migrate_from_file(
            &user_id,
            KEYCHAIN_MASTER_KEY,
            &self.key_path,
        ) {
            Ok(crate::openhuman::keyring::MigrationOutcome::MigratedAndDeleted) => {
                log::info!(
                    "[security] migrated legacy secret-store key into keychain from {}",
                    self.key_path.display()
                );
            }
            Ok(crate::openhuman::keyring::MigrationOutcome::AlreadyMigrated)
            | Ok(crate::openhuman::keyring::MigrationOutcome::NoSourceFile) => {}
            Err(error) => {
                log::warn!(
                    "[security] failed to migrate legacy secret-store key from {}: {error}",
                    self.key_path.display()
                );
            }
        }

        let hex_key =
            crate::openhuman::keyring::get_or_create_random(&user_id, KEYCHAIN_MASTER_KEY, KEY_LEN)
                .map_err(|e| {
                    anyhow::anyhow!("Failed to load secret-store master key from keychain: {e}")
                })?;
        decode_key_hex(hex_key.trim())
    }

    /// Load the encryption key from keychain-backed storage, falling back to
    /// the legacy file path only in unit tests.
    ///
    /// The decoded key is cached process-wide keyed by `key_path`, so repeated
    /// callers (e.g. every `app_state_snapshot` poll) hit memory instead of
    /// disk/keychain lookup.
    fn load_or_create_key(&self) -> Result<Vec<u8>> {
        // Normalize the path once so all callers share the same cache slot
        // regardless of how `key_path` was spelled (relative vs absolute,
        // symlinks, case-variants on Windows).
        let cache_key_path = normalize_cache_path(&self.key_path);

        if let Some(cached) = cached_key(&cache_key_path) {
            return Ok(cached);
        }

        if !self.use_legacy_file_key() {
            let key = self.load_or_create_key_from_keyring()?;
            cache_key(&cache_key_path, &key);
            return Ok(key);
        }

        if self.key_path.exists() {
            let read_result = read_key_file_with_retry(&self.key_path);

            // On Windows a previous bad icacls invocation may have stripped the
            // inherited ACEs from %APPDATA% without granting the current user
            // explicit access, leaving the file permanently unreadable. Attempt
            // a one-shot ACL repair via `icacls /reset` before giving up.
            #[cfg(windows)]
            let read_result = if let Err(ref e) = read_result {
                if is_permission_error(e) {
                    log::warn!(
                        "[security] PermissionDenied reading key file '{}'; \
                         attempting icacls /reset self-repair",
                        self.key_path.display()
                    );
                    repair_windows_acl(&self.key_path);
                    // Single retry regardless of whether repair reported success —
                    // icacls /reset may partially restore access even on a non-zero exit.
                    read_key_file_with_retry(&self.key_path)
                } else {
                    read_result
                }
            } else {
                read_result
            };

            let hex_key = read_result.with_context(|| {
                let mut msg = format!(
                    "Failed to read secret key file at {}",
                    self.key_path.display()
                );
                #[cfg(windows)]
                {
                    msg.push_str(
                        "\n\nThis is often caused by incorrect file permissions on Windows. \
                         Try repairing ACLs on the .openhuman directory:\n\
                         icacls \"%USERPROFILE%\\.openhuman\" /reset /t /c\n\
                         icacls \"%USERPROFILE%\\.openhuman\\.secret_key\" /reset /c",
                    );
                }
                msg
            })?;
            let key = decode_key_hex(hex_key.trim())?;
            cache_key(&cache_key_path, &key);
            Ok(key)
        } else {
            let key = generate_random_key();
            let hex_key = hex_encode(&key);
            if let Some(parent) = self.key_path.parent() {
                fs::create_dir_all(parent)?;
            }

            // Write key file with restrictive permissions atomically on Unix
            // to avoid a TOCTOU race where the file is briefly world-readable,
            // and to avoid clobbering a key another process created first.
            // See: src/core/auth.rs:write_token_file for the reference pattern.
            #[cfg(unix)]
            {
                use std::io::Write as _;
                use std::os::unix::fs::OpenOptionsExt as _;
                let open_result = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&self.key_path);

                match open_result {
                    Ok(mut file) => {
                        file.write_all(hex_key.as_bytes())
                            .context("Failed to write secret key file")?;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        let existing_key = read_key_file_with_retry(&self.key_path)
                            .with_context(|| {
                                format!(
                                    "Secret key file was created concurrently but could not be read at {}",
                                    self.key_path.display()
                                )
                            })
                            .and_then(|existing_hex| decode_key_hex(existing_hex.trim()))?;
                        cache_key(&cache_key_path, &existing_key);
                        return Ok(existing_key);
                    }
                    Err(error) => {
                        return Err(error).context("Failed to create secret key file");
                    }
                }
            }
            #[cfg(not(unix))]
            {
                use std::io::Write as _;

                let open_result = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&self.key_path);

                match open_result {
                    Ok(mut file) => {
                        file.write_all(hex_key.as_bytes())
                            .context("Failed to write secret key file")?;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        let existing_key = read_key_file_with_retry(&self.key_path)
                            .with_context(|| {
                                format!(
                                    "Secret key file was created concurrently but could not be read at {}",
                                    self.key_path.display()
                                )
                            })
                            .and_then(|existing_hex| decode_key_hex(existing_hex.trim()))?;
                        cache_key(&cache_key_path, &existing_key);
                        return Ok(existing_key);
                    }
                    Err(error) => {
                        return Err(error).context("Failed to create secret key file");
                    }
                }
            }
            #[cfg(windows)]
            {
                // On Windows, use icacls to restrict permissions to current user only.
                // We use USERDOMAIN\USERNAME so the account is resolved correctly on
                // domain-joined and AAD-joined machines (bare USERNAME is ambiguous and
                // may refer to a local account that doesn't match the signed-in user).
                let username = std::env::var("USERNAME").unwrap_or_default();
                let userdomain = std::env::var("USERDOMAIN").unwrap_or_default();
                let computername = std::env::var("COMPUTERNAME").unwrap_or_default();
                let qualified_username =
                    qualify_windows_username(&username, &userdomain, &computername);
                let Some(grant_arg) = build_windows_icacls_grant_arg(&qualified_username) else {
                    log::warn!(
                        "[security] USERNAME/USERDOMAIN environment variables are empty; \
                         cannot restrict key file permissions via icacls"
                    );
                    cache_key(&cache_key_path, &key);
                    return Ok(key);
                };

                let icacls_ok = match std::process::Command::new("icacls")
                    .arg(&self.key_path)
                    .args(["/inheritance:r", "/grant:r"])
                    .arg(&grant_arg)
                    .output()
                {
                    Ok(o) if o.status.success() => {
                        log::debug!("[security] key file permissions restricted via icacls");
                        true
                    }
                    Ok(o) => {
                        log::warn!(
                            "[security] icacls exited {:?} for account '{}'; \
                             restoring inherited ACLs so the file remains readable",
                            o.status.code(),
                            grant_arg,
                        );
                        false
                    }
                    Err(e) => {
                        log::warn!(
                            "[security] could not run icacls: {e}; \
                                    restoring inherited ACLs"
                        );
                        false
                    }
                };
                // If the icacls grant command failed, the `/inheritance:r` flag may have
                // already stripped the inherited ACEs that let the current user read the
                // file. Explicitly reset to restore inheritance so the file is always
                // readable — a slightly weaker ACL is preferable to a locked-out user.
                if !icacls_ok {
                    let _ = std::process::Command::new("icacls")
                        .arg(&self.key_path)
                        .args(["/reset"])
                        .output();
                }
            }

            cache_key(&cache_key_path, &key);
            Ok(key)
        }
    }
}

fn keyring_user_id_from_dir(openhuman_dir: &Path) -> String {
    if let Some(id) = openhuman_dir.file_name().and_then(|s| s.to_str()) {
        if !id.is_empty() {
            return id.to_string();
        }
    }

    let path_str = openhuman_dir.to_string_lossy();
    let mut hash: u64 = 14695981039346656037u64;
    for b in path_str.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(1099511628211u64);
    }
    format!("secretstore-path-{hash:016x}")
}

/// Normalize a path into a stable cache key. Tries `canonicalize` first (so
/// symlinks, relative paths, and Windows case-variants all collapse to the
/// same key), falls back to `std::path::absolute` when the file does not yet
/// exist (e.g. the create branch in `load_or_create_key`), and finally to the
/// raw path so a normalization failure never breaks the cache.
fn normalize_cache_path(path: &Path) -> PathBuf {
    fs::canonicalize(path)
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

/// Process-wide cache of decoded key bytes keyed by absolute path.
///
/// Loading the key once per process is both faster and more reliable than
/// re-reading `.secret_key` on every decrypt. On Windows the file can be
/// transiently inaccessible (AV scanners holding a handle), and re-reading
/// turned that transient failure into a perma-failure for every subsequent
/// RPC call.
fn key_cache() -> &'static Mutex<HashMap<PathBuf, Vec<u8>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Vec<u8>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_key(path: &Path) -> Option<Vec<u8>> {
    key_cache().lock().ok()?.get(path).cloned()
}

fn cache_key(path: &Path, key: &[u8]) {
    if let Ok(mut cache) = key_cache().lock() {
        cache.insert(path.to_path_buf(), key.to_vec());
    }
}

/// Clear the cached key for `path`. Test-only — production code should never
/// need to invalidate the cache, since the key file is write-once.
#[cfg(test)]
pub(super) fn clear_cached_key(path: &Path) {
    let normalized = normalize_cache_path(path);
    if let Ok(mut cache) = key_cache().lock() {
        cache.remove(&normalized);
    }
}

/// Read the key file, retrying transient errors a handful of times.
///
/// Windows AV scanners (Defender, etc.) routinely hold short-lived read
/// handles right after a file is created, which surfaces as
/// `ERROR_SHARING_VIOLATION` (raw OS error 32) or `PermissionDenied`. A few
/// short backoffs are enough to ride over the lock; the typical successful
/// path returns on the first attempt with zero added latency.
fn read_key_file_with_retry(path: &Path) -> std::io::Result<String> {
    use std::io::ErrorKind;

    const MAX_ATTEMPTS: u32 = 5;
    let mut last_err: Option<std::io::Error> = None;
    for attempt in 0..MAX_ATTEMPTS {
        match fs::read_to_string(path) {
            Ok(contents) => return Ok(contents),
            Err(err) => {
                let transient = matches!(
                    err.kind(),
                    ErrorKind::PermissionDenied | ErrorKind::Interrupted | ErrorKind::WouldBlock
                ) || err.raw_os_error() == Some(32); // ERROR_SHARING_VIOLATION (Windows)
                last_err = Some(err);
                if !transient || attempt + 1 == MAX_ATTEMPTS {
                    break;
                }
                let backoff_ms = 10u64 << attempt; // 10, 20, 40, 80 ms
                std::thread::sleep(Duration::from_millis(backoff_ms));
            }
        }
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::other("read_to_string failed")))
}

/// Returns `true` when an `std::io::Error` is a permanent permission/access
/// denial rather than a transient sharing violation.
#[cfg(windows)]
fn is_permission_error(e: &std::io::Error) -> bool {
    matches!(e.kind(), std::io::ErrorKind::PermissionDenied) || e.raw_os_error() == Some(5)
    // ERROR_ACCESS_DENIED
}

/// Attempt to repair a locked key file by running `icacls /reset` on it.
///
/// Attempt to repair a locked key file.
///
/// Two-step process:
///   1. `icacls /reset` — removes all explicit ACEs and re-enables ACL
///      inheritance from the parent directory.
///   2. `icacls /grant:r <DOMAIN\USER>:F` — explicit grant for the current
///      user as a belt-and-suspenders fallback for environments (e.g. CI
///      temp dirs) where the parent's inheritance chain may not include the
///      runner account.
///
/// Returns `true` if the file is actually readable after the repair attempt,
/// regardless of which step(s) succeeded.
#[cfg(windows)]
pub(super) fn repair_windows_acl(path: &Path) -> bool {
    // Step 1: restore inheritance.
    match std::process::Command::new("icacls")
        .arg(path)
        .args(["/reset"])
        .output()
    {
        Ok(o) if o.status.success() => {
            log::info!(
                "[security] icacls /reset succeeded for '{}'; ACL inheritance restored",
                path.display()
            );
        }
        Ok(o) => {
            log::warn!(
                "[security] icacls /reset exited {:?} for '{}'",
                o.status.code(),
                path.display()
            );
        }
        Err(e) => {
            log::warn!(
                "[security] could not run icacls /reset for '{}': {e}",
                path.display()
            );
        }
    }

    // Step 2: explicit grant for current user — handles CI environments
    // where the temp/app dir's inheritable ACEs don't include the runner.
    let username = std::env::var("USERNAME").unwrap_or_default();
    let userdomain = std::env::var("USERDOMAIN").unwrap_or_default();
    let computername = std::env::var("COMPUTERNAME").unwrap_or_default();
    let qualified = qualify_windows_username(&username, &userdomain, &computername);
    if let Some(grant_arg) = build_windows_icacls_grant_arg(&qualified) {
        match std::process::Command::new("icacls")
            .arg(path)
            .args(["/grant:r"])
            .arg(&grant_arg)
            .output()
        {
            Ok(o) if o.status.success() => {
                log::debug!(
                    "[security] explicit grant '{grant_arg}' succeeded during repair of '{}'",
                    path.display()
                );
            }
            Ok(o) => {
                log::warn!(
                    "[security] explicit grant '{grant_arg}' exited {:?} during repair of '{}'",
                    o.status.code(),
                    path.display()
                );
            }
            Err(e) => {
                log::warn!(
                    "[security] could not run icacls /grant during repair of '{}': {e}",
                    path.display()
                );
            }
        }
    }

    // Return whether the file is actually readable now — callers use this
    // for logging/metrics; the retry in load_or_create_key is unconditional.
    std::fs::read(path).is_ok()
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

fn decode_key_hex(hex_key: &str) -> Result<Vec<u8>> {
    let key = hex_decode(hex_key).context("Secret key file is corrupt")?;
    anyhow::ensure!(
        key.len() == KEY_LEN,
        "Secret key file has wrong length: expected {KEY_LEN} bytes, got {}",
        key.len()
    );
    Ok(key)
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

/// Produce a domain-qualified Windows account name suitable for `icacls`.
///
/// On domain-joined machines `USERDOMAIN` is the domain name and differs from
/// `COMPUTERNAME`.  On standalone machines they are equal, so we use the bare
/// `username` in that case to avoid a redundant `DESKTOP-XYZ\alice` prefix.
///
/// Returns an empty string when both inputs are empty (caller must treat this
/// as "cannot determine account name").
#[cfg(windows)]
fn qualify_windows_username(username: &str, userdomain: &str, computername: &str) -> String {
    let username = username.trim();
    let userdomain = userdomain.trim();
    let computername = computername.trim();

    if username.is_empty() {
        return String::new();
    }

    // If USERDOMAIN is set and differs from COMPUTERNAME the machine is
    // domain/AAD-joined; use the fully-qualified form so icacls resolves the
    // account unambiguously.
    if !userdomain.is_empty()
        && !computername.is_empty()
        && !userdomain.eq_ignore_ascii_case(computername)
    {
        format!("{userdomain}\\{username}")
    } else {
        username.to_string()
    }
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
#[path = "encrypted_store_tests.rs"]
mod tests;
