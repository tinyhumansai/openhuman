//! Construction and the public CRUD surface of [`AuthProfilesStore`].

use anyhow::Result;
use chrono::Utc;
use std::path::Path;

#[cfg(test)]
use std::sync::atomic::{AtomicBool, AtomicUsize};
#[cfg(test)]
use std::sync::Arc;

use super::{is_lock_create_unwritable_fs, AuthProfile, AuthProfilesData, AuthProfilesStore};
use crate::security::secret_storage::SecretStore;

use super::LOCK_FILENAME;

impl AuthProfilesStore {
    pub fn new(state_dir: &Path, encrypt_secrets: bool) -> Self {
        let user_id = super::user_id_from_state_dir(state_dir);
        let policy = crate::security::keyring_consent::policy::check_secret_access();
        let use_keychain = policy == crate::security::keyring_consent::PolicyDecision::Proceed
            && crate::security::secret_storage::is_available();
        log::debug!(
            "[auth] AuthProfilesStore::new state_dir={} user_id={user_id} use_keychain={use_keychain} policy={policy:?}",
            state_dir.display()
        );
        match policy {
            crate::security::keyring_consent::PolicyDecision::Proceed => {
                if !use_keychain {
                    // OS keychain unavailable despite Proceed policy (probe failed).
                    log::info!(
                        "[auth] OS keychain unavailable — using encrypted JSON for auth profiles user_id={user_id}"
                    );
                }
            }
            crate::security::keyring_consent::PolicyDecision::ConsentRequired => {
                log::warn!(
                    "[auth] keyring consent has not been given — secrets will NOT be persisted \
                     to the OS keychain until the user grants consent. \
                     Falling back to encrypted JSON for auth profiles user_id={user_id}"
                );
            }
            crate::security::keyring_consent::PolicyDecision::Declined => {
                log::warn!(
                    "[auth] user explicitly declined OS keychain storage — \
                     using encrypted JSON for auth profiles user_id={user_id}"
                );
            }
        }
        Self {
            path: super::profiles_file(state_dir),
            lock_path: state_dir.join(LOCK_FILENAME),
            secret_store: SecretStore::new(state_dir, encrypt_secrets),
            user_id,
            use_keychain,
            #[cfg(test)]
            force_transient_failures_write: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            force_transient_failures_rename: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            force_lock_unwritable: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<AuthProfilesData> {
        match self.acquire_lock() {
            Ok(_lock) => self.load_locked(),
            Err(e) if is_lock_create_unwritable_fs(&e) => {
                // RCA Sentry TAURI-RUST-4SZ: a full / read-only filesystem
                // can't create the exclusive lock file, but the store already
                // exists and writers publish via atomic tmp+rename, so a
                // lock-free read is still consistent. The read path is the
                // hot caller here (`app_state_snapshot` polls it every tick),
                // so failing it strands the UI AND floods Sentry once per
                // poll. Degrade to a lock-free read-only load instead — the
                // user keeps their session view, and because no error is
                // produced the noise stops at the source rather than being
                // suppressed downstream. Opportunistic migrations are skipped
                // (they couldn't persist on a full disk anyway).
                log::warn!(
                    "[auth] auth-profile lock could not be created ({e}); \
                     serving lock-free read-only load (likely disk full / read-only FS)"
                );
                self.load_unlocked_readonly()
            }
            Err(e) => Err(e),
        }
    }

    pub fn upsert_profile(&self, mut profile: AuthProfile, set_active: bool) -> Result<()> {
        let _lock = self.acquire_lock()?;
        let mut data = self.load_locked()?;

        profile.updated_at = Utc::now();
        if let Some(existing) = data.profiles.get(&profile.id) {
            profile.created_at = existing.created_at;
        }

        if set_active {
            data.active_profiles
                .insert(profile.provider.clone(), profile.id.clone());
        }

        data.profiles.insert(profile.id.clone(), profile);
        data.updated_at = Utc::now();

        self.save_locked(&data)
    }

    pub fn remove_profile(&self, profile_id: &str) -> Result<bool> {
        let _lock = self.acquire_lock()?;
        let mut data = self.load_locked()?;

        let removed = data.profiles.remove(profile_id).is_some();
        if !removed {
            return Ok(false);
        }

        data.active_profiles
            .retain(|_, active| active != profile_id);
        data.updated_at = Utc::now();
        self.save_locked(&data)?;

        // Clean up keychain entry for this profile (idempotent if keychain
        // is unavailable or no entry exists).
        if self.use_keychain {
            self.keychain_delete_secrets(profile_id);
        }

        Ok(true)
    }

    pub fn set_active_profile(&self, provider: &str, profile_id: &str) -> Result<()> {
        let _lock = self.acquire_lock()?;
        let mut data = self.load_locked()?;

        if !data.profiles.contains_key(profile_id) {
            anyhow::bail!("Auth profile not found: {profile_id}");
        }

        data.active_profiles
            .insert(provider.to_ascii_lowercase(), profile_id.to_string());
        data.updated_at = Utc::now();
        self.save_locked(&data)
    }

    pub fn clear_active_profile(&self, provider: &str) -> Result<()> {
        let _lock = self.acquire_lock()?;
        let mut data = self.load_locked()?;
        data.active_profiles.remove(&provider.to_ascii_lowercase());
        data.updated_at = Utc::now();
        self.save_locked(&data)
    }

    pub fn update_profile<F>(&self, profile_id: &str, mut updater: F) -> Result<AuthProfile>
    where
        F: FnMut(&mut AuthProfile) -> Result<()>,
    {
        let _lock = self.acquire_lock()?;
        let mut data = self.load_locked()?;

        let profile = data
            .profiles
            .get_mut(profile_id)
            .ok_or_else(|| anyhow::anyhow!("Auth profile not found: {profile_id}"))?;

        updater(profile)?;
        profile.updated_at = Utc::now();
        let updated_profile = profile.clone();
        data.updated_at = Utc::now();
        self.save_locked(&data)?;
        Ok(updated_profile)
    }

    pub(super) fn load_locked(&self) -> Result<AuthProfilesData> {
        self.load_resolved(true)
    }

    /// Lock-free read-only load used as the [`AuthProfilesStore::load`]
    /// fallback when the exclusive lock can't be created because the
    /// filesystem won't accept the lock file (disk full / read-only mount —
    /// Sentry TAURI-RUST-4SZ). Safe without the lock because writers publish
    /// the store atomically (tmp + `fs::rename`), so a bare read always sees
    /// a complete file. Skips the opportunistic migration / dropped-profile
    /// rewrite that `load_locked` performs — that write needs both the lock
    /// and a writable disk, and this path runs precisely when neither holds.
    pub(super) fn load_unlocked_readonly(&self) -> Result<AuthProfilesData> {
        self.load_resolved(false)
    }
}
