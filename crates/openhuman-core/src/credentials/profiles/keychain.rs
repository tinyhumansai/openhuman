//! OS-keychain backed secret storage for individual auth profiles.

use anyhow::Context;
#[cfg(feature = "security-module")]
use std::time::Duration;

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
        let name = self.keyring_name(&key);
        self.keyring_operation(tinysecurity_bus::KeyringOperation2::Set {
            name,
            value: payload.into_bytes(),
        })
        .with_context(|| format!("TinySecurity keyring set failed for profile {}", profile.id))?;
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
    pub(super) fn keychain_load_secrets(
        &self,
        profile_id: &str,
    ) -> anyhow::Result<Option<KeychainSecrets>> {
        let key = self.keychain_key_for_profile(profile_id);
        let name = self.keyring_name(&key);
        let response = self
            .keyring_operation(tinysecurity_bus::KeyringOperation2::Get { name: name.clone() })
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
        Ok(Some(secrets))
    }

    /// Delete keychain secrets for a profile (called on profile removal).
    pub(super) fn keychain_delete_secrets(&self, profile_id: &str) -> anyhow::Result<()> {
        let key = self.keychain_key_for_profile(profile_id);
        let name = self.keyring_name(&key);
        if let Err(e) = self.keyring_operation(tinysecurity_bus::KeyringOperation2::Delete { name })
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

    fn keyring_name(&self, logical_key: &str) -> String {
        namespaced_keyring_name(&self.user_id, logical_key)
    }

    fn keyring_operation(
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
            let outcome =
                crate::modules::security::sync_seam::run(Duration::from_secs(125), async move {
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
                });
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
