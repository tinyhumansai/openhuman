//! Non-mutating auth-profile snapshots for doctor checks.

use zeroize::Zeroizing;

use crate::config::Config;
use crate::security::secret_storage::{EncryptedFileBackend, SecretStore};

use super::{
    normalize_profile_id_provider, parse_datetime_with_fallback, parse_optional_datetime,
    parse_profile_kind, profile_id, AuthProfile, AuthProfileKind, KeychainSecrets,
    PersistedAuthProfile, PersistedAuthProfiles, TokenSet, CURRENT_SCHEMA_VERSION,
    PROFILES_FILENAME,
};
use crate::security::credentials::default_profile_id;

/// Safe failure categories for a read-only auth-profile snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOnlyAuthProfileError {
    /// The selected backend, encryption key, or stored record cannot be read safely.
    Unavailable,
    /// The selected profile's persisted auth document is malformed or unsupported.
    InvalidStore,
}

/// Reads one selected profile from its config directory and profile-scoped
/// encrypted-file backend, without locks, migrations, writes, or global
/// storage/backend lookups.
pub(super) fn load_profile(
    config: &Config,
    provider: &str,
    profile_override: Option<&str>,
) -> Result<Option<AuthProfile>, ReadOnlyAuthProfileError> {
    let state_dir = config
        .config_path
        .parent()
        .ok_or(ReadOnlyAuthProfileError::Unavailable)?;
    let path = state_dir.join(PROFILES_FILENAME);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(ReadOnlyAuthProfileError::Unavailable),
    };
    let persisted: PersistedAuthProfiles =
        serde_json::from_slice(&bytes).map_err(|_| ReadOnlyAuthProfileError::InvalidStore)?;
    if persisted.schema_version > CURRENT_SCHEMA_VERSION {
        return Err(ReadOnlyAuthProfileError::InvalidStore);
    }

    let provider = provider.trim().to_ascii_lowercase();
    let Some(id) = selected_profile_id(&persisted, &provider, profile_override) else {
        return Ok(None);
    };
    let profile = persisted
        .profiles
        .get(&id)
        .ok_or(ReadOnlyAuthProfileError::InvalidStore)?;

    let user_id = super::user_id_from_state_dir(state_dir);
    let profile_secret =
        read_profile_secret(config, &user_id, &super::auth_profile_secret_key(&id));
    let secrets = match profile_secret {
        Ok(Some(payload)) => Some(
            serde_json::from_str::<KeychainSecrets>(&payload)
                .map_err(|_| ReadOnlyAuthProfileError::InvalidStore)?,
        ),
        Ok(None) => None,
        Err(_) if has_json_secret_fields(profile) => None,
        Err(error) => return Err(error),
    };

    let encrypted_json = has_encrypted_json_fields(profile);
    let key = if encrypted_json {
        Some(Zeroizing::new(
            read_profile_secret(config, &user_id, "secretstore.master_key")?
                .ok_or(ReadOnlyAuthProfileError::Unavailable)?,
        ))
    } else {
        None
    };
    let store = SecretStore::new(state_dir, config.secrets.encrypt);
    let decrypt = |value: Option<&str>| -> Result<Option<String>, ReadOnlyAuthProfileError> {
        value
            .map(|value| {
                if let Some(key) = key.as_deref() {
                    SecretStore::decrypt_with_existing_key(value, key)
                        .map_err(|_| ReadOnlyAuthProfileError::Unavailable)
                } else if value.starts_with("enc:") || value.starts_with("enc2:") {
                    Err(ReadOnlyAuthProfileError::Unavailable)
                } else {
                    store
                        .decrypt(value)
                        .map_err(|_| ReadOnlyAuthProfileError::Unavailable)
                }
            })
            .transpose()
    };

    let kind =
        parse_profile_kind(&profile.kind).map_err(|_| ReadOnlyAuthProfileError::InvalidStore)?;
    let (access_token, refresh_token, id_token, token) = if let Some(secrets) = secrets {
        (
            secrets.access_token,
            secrets.refresh_token,
            secrets.id_token,
            secrets.token,
        )
    } else {
        (
            decrypt(profile.access_token.as_deref())?,
            decrypt(profile.refresh_token.as_deref())?,
            decrypt(profile.id_token.as_deref())?,
            decrypt(profile.token.as_deref())?,
        )
    };

    let token_set = match kind {
        AuthProfileKind::OAuth => Some(TokenSet {
            access_token: access_token.ok_or(ReadOnlyAuthProfileError::InvalidStore)?,
            refresh_token,
            id_token,
            expires_at: parse_optional_datetime(profile.expires_at.as_deref())
                .map_err(|_| ReadOnlyAuthProfileError::InvalidStore)?,
            token_type: profile.token_type.clone(),
            scope: profile.scope.clone(),
        }),
        AuthProfileKind::Token => None,
    };

    Ok(Some(AuthProfile {
        id,
        provider: profile.provider.clone(),
        profile_name: profile.profile_name.clone(),
        kind,
        account_id: profile.account_id.clone(),
        workspace_id: profile.workspace_id.clone(),
        token_set,
        token,
        metadata: profile.metadata.clone(),
        created_at: parse_datetime_with_fallback(&profile.created_at),
        updated_at: parse_datetime_with_fallback(&profile.updated_at),
    }))
}

fn selected_profile_id(
    persisted: &PersistedAuthProfiles,
    provider: &str,
    profile_override: Option<&str>,
) -> Option<String> {
    let provider_key = if provider.starts_with("provider:") {
        provider.to_owned()
    } else {
        format!("provider:{provider}")
    };
    if let Some(profile_override) = profile_override {
        let requested = if profile_override.contains(':') {
            profile_override.to_owned()
        } else {
            profile_id(provider, profile_override)
        };
        return [requested.clone(), format!("provider:{requested}")]
            .into_iter()
            .find(|id| persisted.profiles.contains_key(id));
    }

    if let Some(active) = persisted
        .active_profiles
        .get(provider)
        .or_else(|| persisted.active_profiles.get(&provider_key))
    {
        let normalized = normalize_profile_id_provider(active);
        if persisted.profiles.contains_key(active) {
            return Some(active.clone());
        }
        if persisted.profiles.contains_key(&normalized) {
            return Some(normalized);
        }
    }
    let default = default_profile_id(provider);
    if persisted.profiles.contains_key(&default) {
        return Some(default);
    }
    let namespaced_default = default_profile_id(&provider_key);
    if persisted.profiles.contains_key(&namespaced_default) {
        return Some(namespaced_default);
    }
    persisted.profiles.iter().find_map(|(id, profile)| {
        (profile.provider.eq_ignore_ascii_case(provider)
            || profile.provider.eq_ignore_ascii_case(&provider_key))
        .then(|| id.clone())
    })
}

fn read_profile_secret(
    config: &Config,
    user_id: &str,
    logical_key: &str,
) -> Result<Option<String>, ReadOnlyAuthProfileError> {
    let key = crate::security::secret_storage::ops::namespaced_key(user_id, logical_key);
    EncryptedFileBackend::new(&config.workspace_dir)
        .get_profile_read_only(&key)
        .map_err(|_| ReadOnlyAuthProfileError::Unavailable)
}

fn has_json_secret_fields(profile: &PersistedAuthProfile) -> bool {
    profile.access_token.is_some()
        || profile.refresh_token.is_some()
        || profile.id_token.is_some()
        || profile.token.is_some()
}

fn has_encrypted_json_fields(profile: &PersistedAuthProfile) -> bool {
    [
        &profile.access_token,
        &profile.refresh_token,
        &profile.id_token,
        &profile.token,
    ]
    .into_iter()
    .flatten()
    .any(|value| value.starts_with("enc:") || value.starts_with("enc2:"))
}
