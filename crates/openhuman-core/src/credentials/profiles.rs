use crate::security::secret_storage::SecretStore;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[allow(unused_imports)]
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

mod keychain;
#[cfg(test)]
pub(crate) use keychain::namespaced_keyring_name;
mod lock;
mod migration;
mod persistence;
#[cfg(test)]
pub(crate) use persistence::is_persisted_document;
use persistence::{PersistedAuthProfile, PersistedAuthProfiles};
mod readonly;
pub(crate) use persistence::PROFILE_DOCUMENT_SECRET_NAME;
mod store_core;
pub use readonly::ReadOnlyAuthProfileError;

pub(crate) async fn load_profile_read_only(
    config: &crate::config::Config,
    provider: &str,
    profile_override: Option<&str>,
) -> Result<Option<AuthProfile>, ReadOnlyAuthProfileError> {
    readonly::load_profile(config, provider, profile_override).await
}

const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Compact secret payload stored as a single keychain entry per auth profile.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct KeychainSecrets {
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub id_token: Option<String>,
}
pub(crate) const PROFILES_FILENAME: &str = "auth-profiles.json";
/// Legacy plaintext profile document retained only for one-time import.
pub(crate) fn profiles_file(state_dir: &Path) -> PathBuf {
    state_dir.join("auth-profiles").join("secrets.enc")
}

pub(crate) fn legacy_profiles_file(state_dir: &Path) -> PathBuf {
    state_dir.join(PROFILES_FILENAME)
}
const LOCK_FILENAME: &str = "auth-profiles.lock";
type EncryptedProfileFields = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthProfileKind {
    OAuth,
    Token,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenSet {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub id_token: Option<String>,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

impl TokenSet {
    pub fn is_expiring_within(&self, skew: Duration) -> bool {
        match self.expires_at {
            Some(expires_at) => {
                let now_plus_skew =
                    Utc::now() + chrono::Duration::from_std(skew).unwrap_or_default();
                expires_at <= now_plus_skew
            }
            None => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthProfile {
    pub id: String,
    pub provider: String,
    pub profile_name: String,
    pub kind: AuthProfileKind,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub token_set: Option<TokenSet>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl AuthProfile {
    pub fn new_oauth(provider: &str, profile_name: &str, token_set: TokenSet) -> Self {
        let now = Utc::now();
        let id = profile_id(provider, profile_name);
        Self {
            id,
            provider: provider.to_string(),
            profile_name: profile_name.to_string(),
            kind: AuthProfileKind::OAuth,
            account_id: None,
            workspace_id: None,
            token_set: Some(token_set),
            token: None,
            metadata: BTreeMap::new(),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn new_token(provider: &str, profile_name: &str, token: String) -> Self {
        let now = Utc::now();
        let id = profile_id(provider, profile_name);
        Self {
            id,
            provider: provider.to_string(),
            profile_name: profile_name.to_string(),
            kind: AuthProfileKind::Token,
            account_id: None,
            workspace_id: None,
            token_set: None,
            token: Some(token),
            metadata: BTreeMap::new(),
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthProfilesData {
    pub schema_version: u32,
    pub updated_at: DateTime<Utc>,
    pub active_profiles: BTreeMap<String, String>,
    pub profiles: BTreeMap<String, AuthProfile>,
}

impl Default for AuthProfilesData {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            updated_at: Utc::now(),
            active_profiles: BTreeMap::new(),
            profiles: BTreeMap::new(),
        }
    }
}

/// Prefix used for keychain entries that store auth profile secrets.
/// Full key format (as handled by the keyring module): `"{user_id}:auth:{profile_id}"`.
const KEYCHAIN_AUTH_PREFIX: &str = "auth:";

/// The logical key used for an auth profile's secret payload.
pub(crate) fn auth_profile_secret_key(profile_id: &str) -> String {
    format!("{KEYCHAIN_AUTH_PREFIX}{profile_id}")
}

/// Derive a stable keychain user-id from a state directory path.
///
/// For a typical path like `/home/alice/.openhuman/users/uid-123` this
/// returns `"uid-123"`.  Falls back to a hash of the full path string so
/// the function always returns a non-empty value even for unusual layouts.
fn user_id_from_state_dir(state_dir: &Path) -> String {
    // The user directory is `{root}/users/{user_id}/` — take the last component.
    if let Some(id) = state_dir.file_name().and_then(|s| s.to_str()) {
        if !id.is_empty() {
            return id.to_string();
        }
    }
    // Fallback: use a hex hash of the path so we always get a stable string.
    let path_str = state_dir.to_string_lossy();
    let mut hash: u64 = 14695981039346656037u64; // FNV-1a offset basis
    for b in path_str.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(1099511628211u64);
    }
    format!("path-{hash:016x}")
}

#[derive(Clone)]
pub struct AuthProfilesStore {
    path: PathBuf,
    legacy_path: PathBuf,
    lock_path: PathBuf,
    secret_store: SecretStore,
    /// Opaque user identifier used to namespace keychain entries.
    user_id: String,
    /// Whether the OS keychain is available on this machine.
    /// Cached at construction time to avoid repeated probes.
    use_keychain: bool,
    /// Present only for Config-backed profile stores. No-Config helper APIs
    /// stay on the profile-local encrypted file and never bypass consent.
    security_config: Option<crate::config::Config>,
}
fn parse_profile_kind(value: &str) -> Result<AuthProfileKind> {
    match value {
        "oauth" => Ok(AuthProfileKind::OAuth),
        "token" => Ok(AuthProfileKind::Token),
        other => anyhow::bail!("Unsupported auth profile kind: {other}"),
    }
}

fn profile_kind_to_string(kind: AuthProfileKind) -> &'static str {
    match kind {
        AuthProfileKind::OAuth => "oauth",
        AuthProfileKind::Token => "token",
    }
}

fn parse_optional_datetime(value: Option<&str>) -> Result<Option<DateTime<Utc>>> {
    value.map(parse_datetime).transpose()
}

fn parse_datetime(value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .with_context(|| format!("Invalid RFC3339 timestamp: {value}"))
}

fn parse_datetime_with_fallback(value: &str) -> DateTime<Utc> {
    parse_datetime(value).unwrap_or_else(|_| Utc::now())
}

pub fn profile_id(provider: &str, profile_name: &str) -> String {
    format!("{}:{}", provider.trim(), profile_name.trim())
}

pub fn normalize_profile_id_provider(id: &str) -> String {
    id.rsplit_once(':').map_or_else(
        || id.to_ascii_lowercase(),
        |(provider, name)| format!("{}:{name}", provider.to_ascii_lowercase()),
    )
}

fn quarantine_corrupt_store(path: &Path) -> Result<PathBuf> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("auth-profiles");
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("json");
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut candidate = parent.join(format!("{stem}.corrupt-{ts}.{ext}"));
    let mut suffix = 0u32;
    while candidate.exists() {
        suffix += 1;
        candidate = parent.join(format!("{stem}.corrupt-{ts}-{suffix}.{ext}"));
    }
    fs::rename(path, &candidate).with_context(|| {
        format!(
            "Failed to quarantine corrupt auth profile store {} -> {}",
            path.display(),
            candidate.display()
        )
    })?;
    Ok(candidate)
}

#[cfg(test)]
#[path = "profiles_tests.rs"]
mod tests;
