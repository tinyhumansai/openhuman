//! Reading and writing the persisted auth-profile document through TSD's
//! encrypted local secret store.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;

use tinystoragedrivers_secrets::{EncryptedFileSecrets, SecretStore as _};

use super::{
    profile_kind_to_string, quarantine_corrupt_store, AuthProfile, AuthProfileKind,
    AuthProfilesData, AuthProfilesStore, EncryptedProfileFields, CURRENT_SCHEMA_VERSION,
    PROFILES_FILENAME,
};

/// Whether `bytes` parse as the legacy persisted profiles document.
pub(crate) fn is_persisted_document(bytes: &[u8]) -> bool {
    serde_json::from_slice::<PersistedAuthProfiles>(bytes).is_ok()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PersistedAuthProfiles {
    #[serde(default = "default_schema_version")]
    pub(super) schema_version: u32,
    #[serde(default = "default_now_rfc3339")]
    pub(super) updated_at: String,
    #[serde(default)]
    pub(super) active_profiles: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) profiles: BTreeMap<String, PersistedAuthProfile>,
}

const EMPTY_PROFILE_STORE_UPDATED_AT: &str = "1970-01-01T00:00:00+00:00";

impl Default for PersistedAuthProfiles {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            updated_at: EMPTY_PROFILE_STORE_UPDATED_AT.to_string(),
            active_profiles: BTreeMap::new(),
            profiles: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct PersistedAuthProfile {
    pub(super) provider: String,
    pub(super) profile_name: String,
    pub(super) kind: String,
    #[serde(default)]
    pub(super) account_id: Option<String>,
    #[serde(default)]
    pub(super) workspace_id: Option<String>,
    #[serde(default)]
    pub(super) access_token: Option<String>,
    #[serde(default)]
    pub(super) refresh_token: Option<String>,
    #[serde(default)]
    pub(super) id_token: Option<String>,
    #[serde(default)]
    pub(super) token: Option<String>,
    #[serde(default)]
    pub(super) expires_at: Option<String>,
    #[serde(default)]
    pub(super) token_type: Option<String>,
    #[serde(default)]
    pub(super) scope: Option<String>,
    #[serde(default = "default_now_rfc3339")]
    pub(super) created_at: String,
    #[serde(default = "default_now_rfc3339")]
    pub(super) updated_at: String,
    #[serde(default)]
    pub(super) metadata: BTreeMap<String, String>,
}

fn default_schema_version() -> u32 {
    CURRENT_SCHEMA_VERSION
}

fn default_now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

/// Name of the document inside the profile-local `EncryptedFileSecrets` store.
pub(crate) const PROFILE_DOCUMENT_SECRET_NAME: &str = "auth-profiles";

impl AuthProfilesStore {
    pub(super) async fn prepare_persisted(
        &self,
        data: &AuthProfilesData,
        previous: &PersistedAuthProfiles,
    ) -> Result<PersistedAuthProfiles> {
        let mut persisted = PersistedAuthProfiles {
            schema_version: CURRENT_SCHEMA_VERSION,
            updated_at: data.updated_at.to_rfc3339(),
            active_profiles: data.active_profiles.clone(),
            profiles: BTreeMap::new(),
        };

        for (id, profile) in &data.profiles {
            // The profile document itself is encrypted by TSD. Keep tokens in
            // the keyring when one is available; otherwise the outer encrypted
            // document protects the fields directly.
            let (access_token, refresh_token, id_token, token, expires_at, token_type, scope) =
                if self.use_keychain {
                    // Store secrets in the OS keychain — JSON gets no secret fields.
                    if let Err(e) = self.keychain_store_secrets(profile) {
                        if !super::keychain::should_fallback_to_encrypted_document(&e) {
                            // A per-operation refusal must not be bypassed by
                            // copying the same secret into another store.
                            return Err(e);
                        }
                        // Non-fatal: fall back to fields in the encrypted profile document.
                        log::warn!(
                            "[auth] save: keychain store failed for profile_id={id}: {e}; \
                             falling back to encrypted JSON"
                        );
                        self.encrypt_for_json(profile).await?
                    } else {
                        log::debug!("[auth] save: secrets stored in keychain profile_id={id}");
                        let (expires_at, token_type, scope) = match &profile.token_set {
                            Some(ts) => (
                                ts.expires_at.as_ref().map(DateTime::to_rfc3339),
                                ts.token_type.clone(),
                                ts.scope.clone(),
                            ),
                            None => (None, None, None),
                        };
                        // Secret fields deliberately omitted from JSON.
                        (None, None, None, None, expires_at, token_type, scope)
                    }
                } else {
                    // Headless / no keychain — store in the encrypted document.
                    self.encrypt_for_json(profile).await?
                };

            persisted.profiles.insert(
                id.clone(),
                PersistedAuthProfile {
                    provider: profile.provider.clone(),
                    profile_name: profile.profile_name.clone(),
                    kind: profile_kind_to_string(profile.kind).to_string(),
                    account_id: profile.account_id.clone(),
                    workspace_id: profile.workspace_id.clone(),
                    access_token,
                    refresh_token,
                    id_token,
                    token,
                    expires_at,
                    token_type,
                    scope,
                    metadata: profile.metadata.clone(),
                    created_at: profile.created_at.to_rfc3339(),
                    updated_at: profile.updated_at.to_rfc3339(),
                },
            );
        }

        if !self.use_keychain {
            // Entries whose secrets are only in the unavailable keychain never
            // reach `data`; keep them so they resolve once it reads again.
            for (id, held) in previous.profiles.clone() {
                if !persisted.profiles.contains_key(&id)
                    && super::migration::keychain_held_oauth(&held)
                {
                    persisted.profiles.insert(id, held);
                }
            }
        }

        Ok(persisted)
    }

    /// Prepare a profile's fields for storage inside the encrypted document.
    pub(super) async fn encrypt_for_json(
        &self,
        profile: &AuthProfile,
    ) -> Result<EncryptedProfileFields> {
        let (access_token, refresh_token, id_token, token, expires_at, token_type, scope) =
            match (&profile.kind, &profile.token_set) {
                (AuthProfileKind::OAuth, Some(token_set)) => (
                    Some(token_set.access_token.clone()),
                    token_set.refresh_token.clone(),
                    token_set.id_token.clone(),
                    None,
                    token_set.expires_at.as_ref().map(DateTime::to_rfc3339),
                    token_set.token_type.clone(),
                    token_set.scope.clone(),
                ),
                _ => (None, None, None, profile.token.clone(), None, None, None),
            };
        let access_token = encrypt_profile_field(&self.secret_store, access_token).await?;
        let refresh_token = encrypt_profile_field(&self.secret_store, refresh_token).await?;
        let id_token = encrypt_profile_field(&self.secret_store, id_token).await?;
        let token = encrypt_profile_field(&self.secret_store, token).await?;
        Ok((
            access_token,
            refresh_token,
            id_token,
            token,
            expires_at,
            token_type,
            scope,
        ))
    }

    pub(super) fn read_persisted_locked(&self) -> Result<PersistedAuthProfiles> {
        if let Some(persisted) = self.read_encrypted_file_locked()? {
            return Ok(persisted);
        }

        let legacy = self.read_file_locked()?;
        if self.legacy_path.exists() {
            // Import before returning so a legacy document is never used as
            // the target of a subsequent plaintext write. Keep the source if
            // publication fails; the next read can retry the migration.
            self.write_persisted_locally(&legacy)?;
            let _ = fs::remove_file(&self.legacy_path);
        }
        Ok(legacy)
    }

    /// Reads the legacy root-level `auth-profiles.json` document for one-time import.
    fn read_file_locked(&self) -> Result<PersistedAuthProfiles> {
        if !self.legacy_path.exists() {
            return Ok(PersistedAuthProfiles::default());
        }

        let bytes = fs::read(&self.legacy_path).with_context(|| {
            format!(
                "Failed to read auth profile store at {}",
                self.legacy_path.display()
            )
        })?;

        if bytes.is_empty() {
            return Ok(PersistedAuthProfiles::default());
        }

        let mut persisted: PersistedAuthProfiles = match serde_json::from_slice(&bytes) {
            Ok(p) => p,
            Err(err) => {
                let quarantined = quarantine_corrupt_store(&self.legacy_path)?;
                let quarantined_file = quarantined
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("auth-profiles.corrupt");
                tracing::warn!(
                    path_file = PROFILES_FILENAME,
                    quarantined_file = quarantined_file,
                    error = %err,
                    "[credentials] auth profile store unparseable; quarantined and reset to empty"
                );
                return Ok(PersistedAuthProfiles::default());
            }
        };

        if persisted.schema_version == 0 {
            persisted.schema_version = CURRENT_SCHEMA_VERSION;
        }

        if persisted.schema_version > CURRENT_SCHEMA_VERSION {
            anyhow::bail!(
                "Unsupported auth profile schema version {} (max supported: {})",
                persisted.schema_version,
                CURRENT_SCHEMA_VERSION
            );
        }

        Ok(persisted)
    }

    pub(super) fn write_persisted_locked(&self, persisted: &PersistedAuthProfiles) -> Result<()> {
        self.write_persisted_locally(persisted)?;
        let _ = fs::remove_file(&self.legacy_path);
        Ok(())
    }

    fn read_encrypted_file_locked(&self) -> Result<Option<PersistedAuthProfiles>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let directory = self.path.parent().expect("profile store has parent");
        let store = EncryptedFileSecrets::with_key_file_read_only(directory)
            .context("Failed to open encrypted auth profile store")?;
        let value =
            crate::storage::block_on(async move { store.get(PROFILE_DOCUMENT_SECRET_NAME).await })
                .context("Failed to read encrypted auth profile store")?;
        let Some(value) = value else {
            return Ok(None);
        };
        let persisted = serde_json::from_slice(&value)
            .context("Encrypted auth profile document is not valid JSON")?;
        Ok(Some(persisted))
    }

    fn write_persisted_locally(&self, persisted: &PersistedAuthProfiles) -> Result<()> {
        let json = serde_json::to_vec(persisted).context("Failed to serialize auth profiles")?;
        let directory = self.path.parent().expect("profile store has parent");
        let store = EncryptedFileSecrets::with_key_file(directory)
            .context("Failed to open encrypted auth profile store")?;
        crate::storage::block_on(
            async move { store.set(PROFILE_DOCUMENT_SECRET_NAME, &json).await },
        )
        .context("Failed to write encrypted auth profile store")
    }

    pub(super) async fn decrypt_optional(
        &self,
        value: Option<&str>,
    ) -> Result<(Option<String>, Option<String>)> {
        match value {
            Some(value) if !value.is_empty() => {
                if value.starts_with("enc:") {
                    anyhow::bail!("legacy XOR-encrypted auth profile secrets are unsupported");
                }
                Ok((
                    Some(self.secret_store.decrypt_via_security(value).await?),
                    None,
                ))
            }
            Some(_) | None => Ok((None, None)),
        }
    }
}

async fn encrypt_profile_field(
    store: &super::SecretStore,
    value: Option<String>,
) -> Result<Option<String>> {
    match value {
        Some(value) => store.encrypt_via_security(&value).await.map(Some),
        None => Ok(None),
    }
}
