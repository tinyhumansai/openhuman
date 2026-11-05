//! OS-keychain backed secret storage for individual auth profiles.

use anyhow::Context;

#[derive(Debug)]
pub(super) enum KeyringConsentError {
    Denied,
    Pending,
}

impl std::fmt::Display for KeyringConsentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Denied => f.write_str("human denied this keyring operation"),
            Self::Pending => f.write_str("keyring operation approval timed out"),
        }
    }
}

impl std::error::Error for KeyringConsentError {}

pub(super) fn is_consent_error(error: &anyhow::Error) -> bool {
    error.downcast_ref::<KeyringConsentError>().is_some()
}

pub(super) fn should_fallback_to_encrypted_document(error: &anyhow::Error) -> bool {
    !is_consent_error(error)
}

use super::{AuthProfile, AuthProfilesStore, KeychainSecrets};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

/// Profile secrets this process has already read from or written to the
/// keyring, by profile store and keyring name. Every keyring read asks the
/// user, and the auth state is read on every poll, so without this each poll
/// would prompt again. Held in memory only; a write or delete drops the entry,
/// and a refused or failed read never fills one.
// ponytail: another process changing the same keyring entry is not seen until
// this one restarts; the desktop core is the only writer of its profiles.
static READ_CACHE: OnceLock<Mutex<HashMap<(PathBuf, String), KeychainSecrets>>> = OnceLock::new();

fn read_cache() -> MutexGuard<'static, HashMap<(PathBuf, String), KeychainSecrets>> {
    READ_CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl AuthProfilesStore {
    /// Build a keychain key for an auth profile's combined secret payload.
    pub(super) fn keychain_key_for_profile(&self, profile_id: &str) -> String {
        super::auth_profile_secret_key(profile_id)
    }

    /// Store auth secrets for a profile in the OS keychain.
    ///
    /// The secrets are serialized as a compact JSON object so a single
    /// keychain entry holds all token fields for the profile.
    pub(super) async fn keychain_store_secrets(&self, profile: &AuthProfile) -> anyhow::Result<()> {
        let key = self.keychain_key_for_profile(&profile.id);
        let secrets = KeychainSecrets {
            token: profile.token.clone(),
            access_token: profile.token_set.as_ref().map(|ts| ts.access_token.clone()),
            refresh_token: profile
                .token_set
                .as_ref()
                .and_then(|ts| ts.refresh_token.clone()),
            id_token: profile
                .token_set
                .as_ref()
                .and_then(|ts| ts.id_token.clone()),
        };
        let payload = serde_json::to_string(&secrets)
            .context("Failed to serialize auth secrets for keychain")?;
        let name = self.keyring_name(&key);
        let cache_key = self.read_cache_key(&name);
        read_cache().remove(&cache_key);
        self.keyring_operation(tinysecurity_bus::KeyringOperation2::Set {
            name,
            value: payload.into_bytes(),
        })
        .await
        .with_context(|| format!("TinySecurity keyring set failed for profile {}", profile.id))?;
        read_cache().insert(cache_key, secrets);
        log::debug!(
            "[auth] keychain_store_secrets stored profile_id={} user_id={}",
            profile.id,
            self.user_id
        );
        Ok(())
    }

    /// Load auth secrets for a profile from the OS keychain.
    ///
    /// Returns `None` if no keychain entry exists for the profile, and an error
    /// when the keychain could not be read (locked, denied, unavailable).
    pub(super) async fn keychain_load_secrets(
        &self,
        profile_id: &str,
    ) -> anyhow::Result<Option<KeychainSecrets>> {
        let key = self.keychain_key_for_profile(profile_id);
        let name = self.keyring_name(&key);
        let cache_key = self.read_cache_key(&name);
        if let Some(secrets) = read_cache().get(&cache_key).cloned() {
            log::debug!(
                "[auth] keychain_load_secrets cached profile_id={profile_id} user_id={}",
                self.user_id
            );
            return Ok(Some(secrets));
        }
        let response = self
            .keyring_operation(tinysecurity_bus::KeyringOperation2::Get { name: name.clone() })
            .await
            .with_context(|| {
                format!("TinySecurity keyring read failed for profile {profile_id}")
            })?;
        let payload = match response.found {
            true => String::from_utf8(response.bytes)
                .context("TinySecurity keyring returned a non-UTF-8 profile payload")?,
            false => {
                log::debug!(
                    "[auth] keychain_load_secrets miss profile_id={profile_id} user_id={}",
                    self.user_id
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
        read_cache().insert(cache_key, secrets.clone());
        Ok(Some(secrets))
    }

    /// Delete keychain secrets for a profile (called on profile removal).
    pub(super) async fn keychain_delete_secrets(&self, profile_id: &str) -> anyhow::Result<()> {
        let key = self.keychain_key_for_profile(profile_id);
        let name = self.keyring_name(&key);
        read_cache().remove(&self.read_cache_key(&name));
        if let Err(e) = self
            .keyring_operation(tinysecurity_bus::KeyringOperation2::Delete { name })
            .await
        {
            log::warn!(
                "[auth] keychain_delete_secrets failed profile_id={profile_id} user_id={}: {e}",
                self.user_id,
            );
            return Err(e);
        } else {
            log::debug!(
                "[auth] keychain_delete_secrets ok profile_id={profile_id} user_id={}",
                self.user_id
            );
        }
        Ok(())
    }

    fn read_cache_key(&self, name: &str) -> (PathBuf, String) {
        (self.path.clone(), name.to_owned())
    }

    fn keyring_name(&self, logical_key: &str) -> String {
        namespaced_keyring_name(&self.user_id, logical_key)
    }

    async fn keyring_operation(
        &self,
        operation: tinysecurity_bus::KeyringOperation2,
    ) -> anyhow::Result<tinysecurity_bus::KeyringResponse2> {
        let config = self
            .security_config
            .clone()
            .context("TinySecurity config is unavailable for keyring access")?;
        let caller = tinysecurity_bus::CallerContext {
            agent: "openhuman.credentials".into(),
            thread: None,
            flow: None,
            call_id: uuid::Uuid::new_v4().to_string(),
            origin: tinysecurity_bus::Origin::LocalOperator,
            tier: tinysecurity_bus::AccessTier::Full,
        };
        let request = tinysecurity_bus::KeyringRequest2 {
            caller,
            purpose: "OpenHuman auth profile credential access".into(),
            operation,
        };
        #[cfg(feature = "security-module")]
        {
            let outcome = async move {
                let client = crate::modules::security::client(&config)
                    .await
                    .map_err(|error| error.to_string())?;
                let call_id = request.caller.call_id.clone();
                client.keyring_access2(request).await.map_err(|error| {
                    match crate::security::keyring_consent::operation_approval::
                                take_operation_outcome(&call_id)
                            {
                                Some(
                                    crate::security::keyring_consent::operation_approval::
                                        OperationOutcome::Denied,
                                ) => "openhuman:consent-denied".to_owned(),
                                Some(
                                    crate::security::keyring_consent::operation_approval::
                                        OperationOutcome::Pending,
                                ) => "openhuman:consent-pending".to_owned(),
                                None => error.to_string(),
                            }
                })
            }
            .await;
            outcome.map_err(|error| match error.as_str() {
                "openhuman:consent-denied" => anyhow::Error::new(KeyringConsentError::Denied),
                "openhuman:consent-pending" => anyhow::Error::new(KeyringConsentError::Pending),
                _ => anyhow::Error::msg(error),
            })
        }
        #[cfg(not(feature = "security-module"))]
        {
            let _ = (config, request);
            Err(anyhow::anyhow!("TinySecurity is not built into this host"))
        }
    }
}

pub(crate) fn namespaced_keyring_name(user_id: &str, logical_key: &str) -> String {
    format!("{user_id}:{logical_key}")
}

#[cfg(test)]
#[path = "keychain_tests.rs"]
mod tests;
