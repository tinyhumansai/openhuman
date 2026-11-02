// TinySecurity-backed secret cipher adapter. Key storage and legacy key-file
// migration remain host responsibilities; all cipher operations cross the
// callback-bound TinySecurity bus.

use anyhow::Result;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

/// Manages encrypted storage of secrets (API keys, tokens, etc.)
#[derive(Debug, Clone)]
pub struct SecretStore {
    /// Legacy path to the on-disk key file (`{data_dir}/openhuman/.secret_key`).
    /// Used for one-time migration into the keychain.
    key_path: PathBuf,
    /// Whether encryption is enabled
    enabled: bool,
    /// The exact host configuration used to admit the callback-bound module.
    config: Option<crate::config::Config>,
}

enum SecretCipherOutput {
    Ciphertext(String),
    Plaintext {
        value: Zeroizing<String>,
        migrated: bool,
        replacement_ciphertext: Option<String>,
    },
}

impl SecretStore {
    /// Create a new secret store rooted at the given directory.
    pub fn new(openhuman_dir: &Path, enabled: bool) -> Self {
        Self {
            key_path: openhuman_dir.join(".secret_key"),
            enabled,
            config: None,
        }
    }

    /// Create a store bound to the configuration whose callback authority it uses.
    pub fn new_with_config(
        openhuman_dir: &Path,
        enabled: bool,
        config: &crate::config::Config,
    ) -> Self {
        Self {
            key_path: openhuman_dir.join(".secret_key"),
            enabled,
            config: Some(config.clone()),
        }
    }

    /// Encrypt through the admitted TinySecurity cipher bus.
    pub async fn encrypt_via_security(&self, plaintext: &str) -> Result<String> {
        if !self.enabled || plaintext.is_empty() {
            return Ok(plaintext.to_owned());
        }
        match self
            .run_security_cipher(
                tinysecurity_bus::SecretCipherKind2::Encrypt,
                plaintext.as_bytes(),
                None,
            )
            .await?
        {
            SecretCipherOutput::Ciphertext(value) => Ok(value),
            SecretCipherOutput::Plaintext { .. } => {
                anyhow::bail!("secret encryption was not completed")
            }
        }
    }

    /// Decrypt through TinySecurity; unprefixed values retain plaintext compatibility.
    pub async fn decrypt_via_security(&self, value: &str) -> Result<String> {
        if !self.enabled || !Self::is_encrypted(value) {
            return Ok(value.to_owned());
        }
        match self
            .run_security_cipher(
                tinysecurity_bus::SecretCipherKind2::Decrypt,
                value.as_bytes(),
                None,
            )
            .await?
        {
            SecretCipherOutput::Plaintext { value, .. } => Ok(value.to_string()),
            SecretCipherOutput::Ciphertext(_) => {
                anyhow::bail!("secret decryption was not completed")
            }
        }
    }

    /// Decrypt and migrate legacy ciphertext through TinySecurity.
    pub async fn decrypt_and_migrate_via_security(
        &self,
        value: &str,
    ) -> Result<(String, Option<String>)> {
        if !self.enabled || !Self::is_encrypted(value) {
            return Ok((value.to_owned(), None));
        }
        match self
            .run_security_cipher(
                tinysecurity_bus::SecretCipherKind2::DecryptAndMigrate,
                value.as_bytes(),
                None,
            )
            .await?
        {
            SecretCipherOutput::Plaintext {
                value,
                migrated,
                replacement_ciphertext,
            } => Ok((
                value.to_string(),
                migrated.then_some(replacement_ciphertext).flatten(),
            )),
            SecretCipherOutput::Ciphertext(_) => {
                anyhow::bail!("secret migration was not completed")
            }
        }
    }

    /// Decrypt one value with an explicitly supplied key through TinySecurity.
    /// The key is exposed to the existing configured User backend for one
    /// serialized resolver read; the override denies writes and is never cached.
    pub async fn decrypt_with_existing_key_via_security(
        &self,
        value: &str,
        key: &str,
    ) -> Result<String> {
        if !Self::is_encrypted(value) {
            return Ok(value.to_owned());
        }
        let operation = if value.starts_with("enc:") {
            tinysecurity_bus::SecretCipherKind2::DecryptAndMigrate
        } else {
            tinysecurity_bus::SecretCipherKind2::Decrypt
        };
        match self
            .run_security_cipher(operation, value.as_bytes(), Some(key))
            .await?
        {
            SecretCipherOutput::Plaintext { value, .. } => Ok(value.to_string()),
            SecretCipherOutput::Ciphertext(_) => {
                anyhow::bail!("secret decryption was not completed")
            }
        }
    }

    async fn run_security_cipher(
        &self,
        operation: tinysecurity_bus::SecretCipherKind2,
        input: &[u8],
        existing_key: Option<&str>,
    ) -> Result<SecretCipherOutput> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("TinySecurity configuration is unavailable"))?;
        #[cfg(feature = "security-module")]
        {
            let scope = crate::modules::ops::security_secret_scope()
                .await
                .ok_or_else(|| anyhow::anyhow!("TinySecurity secret scope is unavailable"))?;
            let consumer = crate::modules::ops::security_cipher_consumer()
                .await
                .ok_or_else(|| anyhow::anyhow!("TinySecurity cipher consumer is unavailable"))?;
            let registration_id = crate::modules::ops::security_secret_registration_id()
                .await
                .ok_or_else(|| {
                    anyhow::anyhow!("TinySecurity secret registration is unavailable")
                })?;
            let binding_identity = crate::modules::ops::security_callback_identity()
                .await
                .ok_or_else(|| anyhow::anyhow!("TinySecurity callback identity is unavailable"))?;
            let client = crate::modules::security::client(config)
                .await
                .map_err(anyhow::Error::new)?;
            let generation = client
                .policy_info()
                .await
                .map_err(anyhow::Error::new)?
                .generation;
            let cache_key = normalize_cache_path(&self.key_path);
            let master_handle = if let Some(existing_key) = existing_key {
                crate::modules::security::resolve_user_master_with_existing_key(
                    &client,
                    &scope,
                    &registration_id,
                    existing_key,
                )
                .await
                .map_err(anyhow::Error::new)?
            } else if let Some(handle) =
                cached_master_handle(&cache_key, &registration_id, &binding_identity, generation)
            {
                handle
            } else {
                let handle = crate::modules::security::resolve_user_master(
                    &client,
                    &scope,
                    &registration_id,
                )
                .await
                .map_err(anyhow::Error::new)?;
                cache_master_handle(
                    &cache_key,
                    &registration_id,
                    &binding_identity,
                    generation,
                    &handle,
                );
                handle
            };
            let output = crate::modules::security::run_secret_cipher(
                &client,
                &scope,
                &consumer,
                &registration_id,
                &master_handle,
                operation,
                input,
            )
            .await
            .map_err(anyhow::Error::new)?;
            Ok(match output {
                crate::modules::security::SecretCipherOutput::Ciphertext(value) => {
                    SecretCipherOutput::Ciphertext(value)
                }
                crate::modules::security::SecretCipherOutput::Plaintext {
                    value,
                    migrated,
                    replacement_ciphertext,
                } => SecretCipherOutput::Plaintext {
                    value,
                    migrated,
                    replacement_ciphertext,
                },
            })
        }
        #[cfg(not(feature = "security-module"))]
        {
            let _ = (config, operation, input, existing_key);
            anyhow::bail!("TinySecurity is not built into this host")
        }
    }

    /// Check if a value uses the legacy `enc:` format that should be migrated.
    pub fn needs_migration(value: &str) -> bool {
        value.starts_with("enc:")
    }

    /// Check if a value is already encrypted (current or legacy format).
    pub fn is_encrypted(value: &str) -> bool {
        value.starts_with("enc2:") || value.starts_with("enc:")
    }
}

struct CachedMasterHandle {
    registration_id: String,
    binding_identity: String,
    generation: u64,
    expires: Instant,
    handle: String,
}

const MASTER_HANDLE_CACHE_TTL: Duration = Duration::from_secs(20);

fn master_handle_cache() -> &'static Mutex<HashMap<PathBuf, CachedMasterHandle>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedMasterHandle>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_master_handle(
    path: &Path,
    registration_id: &str,
    binding_identity: &str,
    generation: u64,
) -> Option<String> {
    let mut cache = master_handle_cache().lock().ok()?;
    cache.retain(|_, value| value.expires > Instant::now());
    cache.get(path).and_then(|value| {
        (value.generation == generation
            && value.registration_id == registration_id
            && value.binding_identity == binding_identity)
            .then(|| value.handle.clone())
    })
}

fn cache_master_handle(
    path: &Path,
    registration_id: &str,
    binding_identity: &str,
    generation: u64,
    handle: &str,
) {
    if let Ok(mut cache) = master_handle_cache().lock() {
        cache.insert(
            path.to_path_buf(),
            CachedMasterHandle {
                registration_id: registration_id.to_owned(),
                binding_identity: binding_identity.to_owned(),
                generation,
                expires: Instant::now() + MASTER_HANDLE_CACHE_TTL,
                handle: handle.to_owned(),
            },
        );
    }
}

pub(crate) fn keyring_user_id_from_dir(openhuman_dir: &Path) -> String {
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
/// same key), then `std::path::absolute`, and finally the raw path so
/// normalization failure never breaks the handle cache.
fn normalize_cache_path(path: &Path) -> PathBuf {
    fs::canonicalize(path)
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
#[path = "encrypted_store_tests.rs"]
mod tests;
