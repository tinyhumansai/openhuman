//! Construction and the public CRUD surface of [`AuthProfilesStore`].

use anyhow::{Context, Result};
use chrono::Utc;
use std::path::Path;

use super::{AuthProfile, AuthProfilesData, AuthProfilesStore, PersistedAuthProfiles};
use crate::security::secret_storage::SecretStore;

use super::LOCK_FILENAME;

impl AuthProfilesStore {
    pub fn new(state_dir: &Path, encrypt_secrets: bool) -> Self {
        Self::construct(state_dir, encrypt_secrets, None)
    }

    pub fn new_with_config(
        state_dir: &Path,
        encrypt_secrets: bool,
        config: &crate::config::Config,
    ) -> Self {
        #[cfg(feature = "security-module")]
        crate::modules::security::sync_seam::arm_current();
        Self::construct(state_dir, encrypt_secrets, Some(config.clone()))
    }

    fn construct(
        state_dir: &Path,
        encrypt_secrets: bool,
        security_config: Option<crate::config::Config>,
    ) -> Self {
        let user_id = super::user_id_from_state_dir(state_dir);
        let use_keychain = security_config.is_some() && cfg!(feature = "security-module");
        log::debug!(
            "[auth] AuthProfilesStore::new state_dir={} user_id={user_id} use_keychain={use_keychain}",
            state_dir.display()
        );
        let secret_store = security_config.as_ref().map_or_else(
            || SecretStore::new(state_dir, encrypt_secrets),
            |config| SecretStore::new_with_config(state_dir, encrypt_secrets, config),
        );
        Self {
            path: super::profiles_file(state_dir),
            legacy_path: super::legacy_profiles_file(state_dir),
            lock_path: state_dir.join(LOCK_FILENAME),
            secret_store,
            user_id,
            use_keychain,
            security_config,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read the encrypted document while holding the local lease, then release
    /// it before resolving secrets through the async security bus. Documents
    /// are published atomically, so this snapshot stays internally consistent.
    pub async fn load(&self) -> Result<AuthProfilesData> {
        let persisted = match self.acquire_read_lock()? {
            Some(_lock) => self.read_persisted_locked()?,
            None => {
                log::warn!(
                    "[auth] profile lease unavailable on a full or read-only filesystem; \
                     serving a lock-free read-only snapshot"
                );
                self.read_persisted_locked()?
            }
        };
        self.load_resolved(persisted).await
    }

    async fn snapshot(&self) -> Result<(PersistedAuthProfiles, Vec<u8>)> {
        let persisted = {
            let _lock = self.acquire_lock()?;
            self.read_persisted_locked()?
        };
        let version = serde_json::to_vec(&persisted).context("serialize auth profile snapshot")?;
        Ok((persisted, version))
    }

    async fn publish_if_current(
        &self,
        expected_version: &[u8],
        data: &AuthProfilesData,
        previous: &PersistedAuthProfiles,
    ) -> Result<bool> {
        // Cipher operations can ask the operator for consent. Prepare the
        // candidate document before reacquiring the non-Send local lease.
        let persisted = self.prepare_persisted(data, previous).await?;
        let _lock = self.acquire_lock()?;
        let current = self.read_persisted_locked()?;
        let current_version =
            serde_json::to_vec(&current).context("serialize current auth profile snapshot")?;
        if current_version != expected_version {
            return Ok(false);
        }
        self.write_persisted_locked(&persisted)?;
        Ok(true)
    }

    async fn update_data<F, T>(&self, mut update: F) -> Result<T>
    where
        F: FnMut(&mut AuthProfilesData) -> Result<T>,
    {
        for _ in 0..8 {
            let (persisted, version) = self.snapshot().await?;
            let mut data = self.load_resolved(persisted.clone()).await?;
            let result = update(&mut data)?;
            data.updated_at = Utc::now();
            if self.publish_if_current(&version, &data, &persisted).await? {
                return Ok(result);
            }
        }
        anyhow::bail!("auth profile store changed repeatedly during update")
    }

    pub async fn upsert_profile(&self, mut profile: AuthProfile, set_active: bool) -> Result<()> {
        self.update_data(|data| {
            profile.updated_at = Utc::now();
            if let Some(existing) = data.profiles.get(&profile.id) {
                profile.created_at = existing.created_at;
            }
            if set_active {
                data.active_profiles
                    .insert(profile.provider.clone(), profile.id.clone());
            }
            data.profiles.insert(profile.id.clone(), profile.clone());
            Ok(())
        })
        .await
    }

    pub async fn remove_profile(&self, profile_id: &str) -> Result<bool> {
        for _ in 0..8 {
            let (persisted_before, version) = self.snapshot().await?;
            let mut data = self.load_resolved(persisted_before.clone()).await?;
            if data.profiles.remove(profile_id).is_none() {
                return Ok(false);
            }
            data.active_profiles
                .retain(|_, active| active != profile_id);
            data.updated_at = Utc::now();
            if !self
                .publish_if_current(&version, &data, &persisted_before)
                .await?
            {
                continue;
            }

            if self.use_keychain {
                if let Err(delete_error) = self.keychain_delete_secrets(profile_id) {
                    return self.rollback_profile_removal(
                        profile_id,
                        &persisted_before,
                        delete_error,
                    );
                }
            }
            return Ok(true);
        }
        anyhow::bail!("auth profile store changed repeatedly during removal")
    }

    pub(super) fn rollback_profile_removal(
        &self,
        profile_id: &str,
        persisted_before: &PersistedAuthProfiles,
        delete_error: anyhow::Error,
    ) -> Result<bool> {
        let _lock = self.acquire_lock()?;
        let current = self.read_persisted_locked()?;
        if !current.profiles.contains_key(profile_id) {
            self.write_persisted_locked(persisted_before)?;
        }
        Err(delete_error.context("profile removal was not completed"))
    }

    pub async fn set_active_profile(&self, provider: &str, profile_id: &str) -> Result<()> {
        self.update_data(|data| {
            if !data.profiles.contains_key(profile_id) {
                anyhow::bail!("Auth profile not found: {profile_id}");
            }
            data.active_profiles
                .insert(provider.to_ascii_lowercase(), profile_id.to_string());
            Ok(())
        })
        .await
    }

    pub async fn clear_active_profile(&self, provider: &str) -> Result<()> {
        self.update_data(|data| {
            data.active_profiles.remove(&provider.to_ascii_lowercase());
            Ok(())
        })
        .await
    }

    pub async fn update_profile<F>(&self, profile_id: &str, mut updater: F) -> Result<AuthProfile>
    where
        F: FnMut(&mut AuthProfile) -> Result<()>,
    {
        self.update_data(|data| {
            let profile = data
                .profiles
                .get_mut(profile_id)
                .ok_or_else(|| anyhow::anyhow!("Auth profile not found: {profile_id}"))?;
            updater(profile)?;
            profile.updated_at = Utc::now();
            Ok(profile.clone())
        })
        .await
    }
}
