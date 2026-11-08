//! OS-keychain backed secret storage for individual auth profiles.

use anyhow::Context;

#[derive(Debug)]
pub(super) enum KeyringConsentError {
    Denied,
    Pending,
    /// Not asked: see [`without_keyring`].
    HeldBack,
}

impl std::fmt::Display for KeyringConsentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Denied => f.write_str("human denied this keyring operation"),
            Self::Pending => f.write_str("keyring operation approval timed out"),
            Self::HeldBack => f.write_str("keyring not asked while no one can approve it"),
        }
    }
}

impl std::error::Error for KeyringConsentError {}

tokio::task_local! {
    /// Present while `future` given to [`without_keyring`] runs.
    static KEYRING_HELD_BACK: ();
}

/// Run `future` without asking the keyring for anything. The keyring is
/// TinySecurity's, so asking it while TinySecurity is still loading waits on
/// that same load; and before the core serves RPC no prompt can be answered.
/// Profiles held there read as unavailable instead, and a write is refused
/// rather than moved into the document.
pub(crate) async fn without_keyring<F: std::future::Future>(future: F) -> F::Output {
    KEYRING_HELD_BACK.scope((), future).await
}

/// Refuses inside [`without_keyring`], before anything is asked or forgotten.
fn refuse_held_back() -> anyhow::Result<()> {
    match KEYRING_HELD_BACK.try_with(|_| ()) {
        Ok(()) => Err(anyhow::Error::new(KeyringConsentError::HeldBack)),
        Err(_) => Ok(()),
    }
}

pub(super) fn is_consent_error(error: &anyhow::Error) -> bool {
    error.downcast_ref::<KeyringConsentError>().is_some()
}

pub(super) fn should_fallback_to_encrypted_document(error: &anyhow::Error) -> bool {
    !is_consent_error(error)
}

use super::{AuthProfile, AuthProfilesStore, KeychainSecrets};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

/// Profile secrets this process has already read from or written to the
/// keyring, by keyring entry name. Every keyring read asks the user, and the
/// auth state is read on every poll, so without this each poll would prompt
/// again. Held in memory only; a write or delete drops the entry, and a
/// refused or failed read never fills one.
// TODO(tinyhumansai/tinysecurity#55): replace with a session-scoped read
// grant in TinySecurity.
// ponytail: another process changing the same keyring entry is not seen until
// this one restarts; the desktop core is the only writer of its profiles.
static READ_CACHE: OnceLock<Mutex<ReadCache>> = OnceLock::new();

#[derive(Default)]
struct ReadCache {
    secrets: HashMap<String, KeychainSecrets>,
    /// Bumped by every write and delete, so a read that started before one
    /// does not cache what it read.
    generations: HashMap<String, u64>,
    /// One cold read per entry at a time; the others wait and take its result.
    gates: HashMap<String, Arc<tokio::sync::Mutex<()>>>,
}

impl ReadCache {
    fn forget(&mut self, name: &str) {
        self.secrets.remove(name);
        *self.generations.entry(name.to_owned()).or_default() += 1;
    }

    fn generation(&self, name: &str) -> u64 {
        self.generations.get(name).copied().unwrap_or_default()
    }
}

fn read_cache() -> MutexGuard<'static, ReadCache> {
    READ_CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The cached secret for keyring entry `name`, or the one `fetch` reads,
/// cached unless the entry was written or deleted meanwhile.
async fn read_through<F>(name: &str, fetch: F) -> anyhow::Result<Option<KeychainSecrets>>
where
    F: std::future::Future<Output = anyhow::Result<Option<KeychainSecrets>>>,
{
    if let Some(secrets) = read_cache().secrets.get(name).cloned() {
        log::debug!("[auth] keyring read served from this process's cache");
        return Ok(Some(secrets));
    }
    // Served from the cache above or not at all: waiting behind another
    // read's gate could mean waiting on a prompt no one can answer.
    refuse_held_back()?;
    let gate = Arc::clone(read_cache().gates.entry(name.to_owned()).or_default());
    let _turn = gate.lock().await;
    let generation = {
        let cache = read_cache();
        if let Some(secrets) = cache.secrets.get(name).cloned() {
            return Ok(Some(secrets));
        }
        cache.generation(name)
    };
    let secrets = fetch.await?;
    if let Some(secrets) = &secrets {
        let mut cache = read_cache();
        if cache.generation(name) == generation {
            cache.secrets.insert(name.to_owned(), secrets.clone());
        }
    }
    Ok(secrets)
}

fn keychain_secrets_of(profile: &AuthProfile) -> KeychainSecrets {
    let token_set = profile.token_set.as_ref();
    KeychainSecrets {
        token: profile.token.clone(),
        access_token: token_set.map(|ts| ts.access_token.clone()),
        refresh_token: token_set.and_then(|ts| ts.refresh_token.clone()),
        id_token: token_set.and_then(|ts| ts.id_token.clone()),
    }
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
        // A refused write leaves the cached secret it never replaced.
        refuse_held_back()?;
        let key = self.keychain_key_for_profile(&profile.id);
        let secrets = keychain_secrets_of(profile);
        let payload = serde_json::to_string(&secrets)
            .context("Failed to serialize auth secrets for keychain")?;
        let name = self.keyring_name(&key);
        read_cache().forget(&name);
        self.keyring_operation(tinysecurity_bus::KeyringOperation2::Set {
            name: name.clone(),
            value: payload.into_bytes(),
        })
        .await
        .with_context(|| format!("TinySecurity keyring set failed for profile {}", profile.id))?;
        let mut cache = read_cache();
        // Again: a read that began after the first forget may hold the old value.
        cache.forget(&name);
        cache.secrets.insert(name, secrets);
        drop(cache);
        log::debug!(
            "[auth] keychain_store_secrets stored profile_id={} user_id={}",
            profile.id,
            self.user_id
        );
        Ok(())
    }

    /// Whether the keyring holds `profile`'s secrets as this process last read
    /// or wrote them. Never asks the keyring.
    pub(super) fn keychain_holds(&self, profile: &AuthProfile) -> bool {
        let name = self.keyring_name(&self.keychain_key_for_profile(&profile.id));
        read_cache().secrets.get(&name) == Some(&keychain_secrets_of(profile))
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
        read_through(&name, self.keychain_get_secrets(profile_id, &name)).await
    }

    async fn keychain_get_secrets(
        &self,
        profile_id: &str,
        name: &str,
    ) -> anyhow::Result<Option<KeychainSecrets>> {
        let response = self
            .keyring_operation(tinysecurity_bus::KeyringOperation2::Get {
                name: name.to_owned(),
            })
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
        Ok(Some(secrets))
    }

    /// Delete keychain secrets for a profile (called on profile removal).
    pub(super) async fn keychain_delete_secrets(&self, profile_id: &str) -> anyhow::Result<()> {
        refuse_held_back()?;
        let key = self.keychain_key_for_profile(profile_id);
        let name = self.keyring_name(&key);
        read_cache().forget(&name);
        let deleted = self
            .keyring_operation(tinysecurity_bus::KeyringOperation2::Delete { name: name.clone() })
            .await;
        read_cache().forget(&name);
        if let Err(e) = deleted {
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

    async fn keyring_operation(
        &self,
        operation: tinysecurity_bus::KeyringOperation2,
    ) -> anyhow::Result<tinysecurity_bus::KeyringResponse2> {
        refuse_held_back().inspect_err(|_| {
            log::debug!(
                "[auth] keyring operation held back user_id={}",
                self.user_id
            )
        })?;
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
