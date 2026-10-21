//! OS-keychain backed secret storage for individual auth profiles.

use anyhow::Context;

use super::{AuthProfile, AuthProfilesStore, KeychainSecrets};

impl AuthProfilesStore {
    /// Build a keychain key for an auth profile's combined secret payload.
    pub(super) fn keychain_key_for_profile(&self, profile_id: &str) -> String {
        super::auth_profile_secret_key(profile_id)
    }

    /// Store auth secrets for a profile in the OS keychain.
    ///
    /// The secrets are serialized as a compact JSON object so a single
    /// keychain entry holds all token fields for the profile.
    pub(super) fn keychain_store_secrets(&self, profile: &AuthProfile) -> anyhow::Result<()> {
        let key = self.keychain_key_for_profile(&profile.id);
        let secrets = serde_json::json!({
            "token": profile.token,
            "access_token": profile.token_set.as_ref().map(|ts| &ts.access_token),
            "refresh_token": profile.token_set.as_ref().and_then(|ts| ts.refresh_token.as_deref()),
            "id_token": profile.token_set.as_ref().and_then(|ts| ts.id_token.as_deref()),
        });
        let payload = serde_json::to_string(&secrets)
            .context("Failed to serialize auth secrets for keychain")?;
        crate::security::secret_storage::set(&self.user_id, &key, &payload).map_err(|e| {
            anyhow::anyhow!(
                "Keychain set failed for profile {}: {e} | detail={}",
                profile.id,
                e.diagnostic()
            )
        })?;
        log::debug!(
            "[auth] keychain_store_secrets stored profile_id={} user_id={}",
            profile.id,
            self.user_id
        );
        Ok(())
    }

    /// Load auth secrets for a profile from the OS keychain.
    ///
    /// Returns `None` if no keychain entry exists for the profile.
    pub(super) fn keychain_load_secrets(
        &self,
        profile_id: &str,
    ) -> anyhow::Result<Option<KeychainSecrets>> {
        let key = self.keychain_key_for_profile(profile_id);
        let payload = match crate::security::secret_storage::get(&self.user_id, &key) {
            Ok(Some(p)) => p,
            Ok(None) => {
                log::debug!(
                    "[auth] keychain_load_secrets miss profile_id={profile_id} user_id={}",
                    self.user_id
                );
                return Ok(None);
            }
            Err(e) => {
                log::warn!(
                    "[auth] keychain_load_secrets error profile_id={profile_id} user_id={}: {e} | detail={}",
                    self.user_id,
                    e.diagnostic()
                );
                return Ok(None);
            }
        };
        let secrets: KeychainSecrets = serde_json::from_str(&payload).map_err(|e| {
            anyhow::anyhow!("Keychain payload for profile {profile_id} is not valid JSON: {e}")
        })?;
        log::debug!(
            "[auth] keychain_load_secrets hit profile_id={profile_id} user_id={}",
            self.user_id
        );
        Ok(Some(secrets))
    }

    /// Delete keychain secrets for a profile (called on profile removal).
    pub(super) fn keychain_delete_secrets(&self, profile_id: &str) {
        let key = self.keychain_key_for_profile(profile_id);
        if let Err(e) = crate::security::secret_storage::delete(&self.user_id, &key) {
            log::warn!(
                "[auth] keychain_delete_secrets error profile_id={profile_id} user_id={}: {e} | detail={}",
                self.user_id,
                e.diagnostic()
            );
        } else {
            log::debug!(
                "[auth] keychain_delete_secrets ok profile_id={profile_id} user_id={}",
                self.user_id
            );
        }
    }
}
