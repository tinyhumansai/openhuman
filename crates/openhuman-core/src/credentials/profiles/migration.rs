//! Shared read + in-memory migration worker for the persisted auth-profile
//! store: dropping unreadable profiles and normalizing profile IDs.

use anyhow::Result;
use chrono::Utc;
use std::collections::BTreeMap;

use super::{
    normalize_profile_id_provider, parse_datetime_with_fallback, parse_optional_datetime,
    parse_profile_kind, profile_id, AuthProfile, AuthProfileKind, AuthProfilesData,
    AuthProfilesStore, PersistedAuthProfile, TokenSet,
};

impl AuthProfilesStore {
    /// Shared read + in-memory resolution worker. Reads the persisted store,
    /// resolves/migrates secrets and drops unrecoverable profiles in memory,
    /// and — only when `persist` is true — writes back any resulting cleanup.
    /// The returned `AuthProfilesData` reflects the in-memory cleanup either
    /// way, so the lock-free read path (`persist = false`) still returns a
    /// correct, fully-resolved view without touching disk.
    pub(super) async fn load_resolved(
        &self,
        mut persisted: super::PersistedAuthProfiles,
    ) -> Result<AuthProfilesData> {
        let mut dropped_ids: Vec<String> = Vec::new();
        let mut pending_keychain_deletes = Vec::new();
        let mut unavailable_keyring_ids = Vec::new();
        let mut profiles = BTreeMap::new();
        for (id, p) in &persisted.profiles {
            // Current secrets may live in the keyring or in the encrypted
            // profile document. Legacy document values are read in place; they
            // are never promoted into the keyring during a profile read.
            let decrypt_document_secrets = || async {
                Ok::<_, anyhow::Error>((
                    self.decrypt_optional(p.access_token.as_deref()).await?.0,
                    self.decrypt_optional(p.refresh_token.as_deref()).await?.0,
                    self.decrypt_optional(p.id_token.as_deref()).await?.0,
                    self.decrypt_optional(p.token.as_deref()).await?.0,
                ))
            };
            let (access_token, refresh_token, id_token, token) = if self.use_keychain {
                match self.keychain_load_secrets(id) {
                    Ok(Some(secrets)) => (
                        secrets.access_token,
                        secrets.refresh_token,
                        secrets.id_token,
                        secrets.token,
                    ),
                    Ok(None) => match decrypt_document_secrets().await {
                        Ok(secrets) => secrets,
                        Err(decrypt_error) => {
                            log::warn!(
                                "[auth] dropping unrecoverable profile provider={}: {decrypt_error}",
                                p.provider
                            );
                            dropped_ids.push(id.clone());
                            continue;
                        }
                    },
                    Err(keyring_error) => {
                        if super::keychain::is_consent_error(&keyring_error) {
                            // An explicit refusal or timeout is authoritative:
                            // do not read a second copy from the profile document.
                            return Err(keyring_error);
                        }
                        if preserve_keyring_only_oauth_after_access_failure(p, true) {
                            unavailable_keyring_ids.push(id.clone());
                        }
                        match decrypt_document_secrets().await {
                            Ok(secrets) => secrets,
                            Err(decrypt_error) => {
                                // The keyring may still contain the only copy.
                                // Keep the store intact until it can be read.
                                log::warn!(
                                    "[auth] keyring and encrypted document unavailable for profile_id={id}: {decrypt_error}"
                                );
                                return Err(keyring_error);
                            }
                        }
                    }
                }
            } else {
                match decrypt_document_secrets().await {
                    Ok(secrets) => secrets,
                    Err(error) => {
                        log::warn!(
                            "[auth] dropping unrecoverable profile provider={}: {error}",
                            p.provider
                        );
                        dropped_ids.push(id.clone());
                        continue;
                    }
                }
            };

            let kind = match parse_profile_kind(&p.kind) {
                Ok(kind) => kind,
                Err(error) => {
                    log::warn!(
                        "[auth] dropping profile with unrecognized kind={:?} provider={}: {error}",
                        p.kind,
                        p.provider
                    );
                    dropped_ids.push(id.clone());
                    continue;
                }
            };
            let token_set = match kind {
                AuthProfileKind::OAuth => {
                    let Some(access_token) = access_token else {
                        if unavailable_keyring_ids.contains(id)
                            || (!self.use_keychain && keychain_held_oauth(p))
                        {
                            continue;
                        }
                        log::warn!(
                            "[auth] dropping OAuth profile with missing access_token: provider={}",
                            p.provider
                        );
                        dropped_ids.push(id.clone());
                        continue;
                    };
                    Some(TokenSet {
                        access_token,
                        refresh_token,
                        id_token,
                        expires_at: parse_optional_datetime(p.expires_at.as_deref())?,
                        token_type: p.token_type.clone(),
                        scope: p.scope.clone(),
                    })
                }
                AuthProfileKind::Token => None,
            };
            profiles.insert(
                id.clone(),
                AuthProfile {
                    id: id.clone(),
                    provider: p.provider.clone(),
                    profile_name: p.profile_name.clone(),
                    kind,
                    account_id: p.account_id.clone(),
                    workspace_id: p.workspace_id.clone(),
                    token_set,
                    token: if kind == AuthProfileKind::Token {
                        token
                    } else {
                        None
                    },
                    metadata: p.metadata.clone(),
                    created_at: parse_datetime_with_fallback(&p.created_at),
                    updated_at: parse_datetime_with_fallback(&p.updated_at),
                },
            );
        }

        if !dropped_ids.is_empty() {
            for id in &dropped_ids {
                persisted.profiles.remove(id);
            }
            persisted
                .active_profiles
                .retain(|_, profile_id| !dropped_ids.contains(profile_id));
            persisted.updated_at = Utc::now().to_rfc3339();
            log::warn!(
                "[auth] purged {} unrecoverable profile(s) from store at {} \
                 (provider list redacted to avoid leaking PII)",
                dropped_ids.len(),
                self.path.display(),
            );
        }

        let mut key_migrated = false;
        let mut new_active = BTreeMap::new();
        let mut active_entries: Vec<_> = persisted.active_profiles.iter().collect();
        active_entries.sort_by_key(|(k, _)| k.to_ascii_lowercase() != **k);
        for (k, v) in active_entries {
            let lower = k.to_ascii_lowercase();
            let normalized_value = normalize_profile_id_provider(v);
            if &lower != k || &normalized_value != v {
                key_migrated = true;
            }
            if new_active.contains_key(&lower) {
                if k == &lower {
                    new_active.insert(lower.clone(), normalized_value.clone());
                }
            } else {
                new_active.insert(lower, normalized_value);
            }
        }
        if key_migrated {
            persisted.active_profiles = new_active;
        }

        let mut new_persisted_profiles: BTreeMap<String, PersistedAuthProfile> = BTreeMap::new();
        let mut new_profiles: BTreeMap<String, AuthProfile> = BTreeMap::new();
        let mut profile_id_migration_targets: BTreeMap<String, String> = BTreeMap::new();
        let mut profile_casing_changed_count: usize = 0;
        let mut profile_migration_conflicts: usize = 0;

        let mut profile_entries: Vec<_> = std::mem::take(&mut persisted.profiles)
            .into_iter()
            .collect();
        profile_entries.sort_by_key(|(id, p)| {
            id != &profile_id(&p.provider.to_ascii_lowercase(), &p.profile_name)
        });
        for (id, mut p) in profile_entries {
            let original_provider = p.provider.clone();
            let lower_provider = p.provider.to_ascii_lowercase();
            let normalized_id = profile_id(&lower_provider, &p.profile_name);
            let provider_or_id_changed = normalized_id != id || lower_provider != p.provider;

            if let Some(mut ap) = profiles.remove(&id) {
                if new_profiles.contains_key(&normalized_id) {
                    profile_migration_conflicts += 1;
                    if id == normalized_id {
                        let old_id = new_profiles
                            .insert(normalized_id.clone(), ap)
                            .map(|old| old.id);
                        new_persisted_profiles.insert(normalized_id.clone(), p);
                        profile_id_migration_targets.insert(id.clone(), normalized_id.clone());
                        profile_id_migration_targets
                            .insert(normalize_profile_id_provider(&id), normalized_id.clone());
                        if self.use_keychain {
                            if let Some(old_id) = &old_id {
                                pending_keychain_deletes.push(old_id.clone());
                            }
                        }
                        log::debug!(
                            "[auth] profile id migration collision: dropped mixed-case profile_id={:?}",
                            old_id
                        );
                    } else {
                        if self.use_keychain {
                            pending_keychain_deletes.push(id.clone());
                        }
                        log::debug!(
                            "[auth] profile id migration collision: dropped mixed-case profile_id={id}"
                        );
                    }
                } else {
                    ap.id = normalized_id.clone();
                    ap.provider = lower_provider.clone();
                    p.provider = lower_provider;

                    let migration_succeeded = if self.use_keychain
                        && provider_or_id_changed
                        && (ap.token.is_some() || ap.token_set.is_some())
                    {
                        match self.keychain_store_secrets(&ap) {
                            Ok(()) => {
                                pending_keychain_deletes.push(id.clone());
                                true
                            }
                            Err(e) => {
                                log::warn!(
                                    "[auth] load: keychain profile-id migration failed old_profile_id={id}: {e}; retaining legacy entry"
                                );
                                false
                            }
                        }
                    } else {
                        true
                    };

                    let final_id = if migration_succeeded {
                        if provider_or_id_changed {
                            profile_casing_changed_count += 1;
                        }
                        normalized_id.clone()
                    } else {
                        ap.id = id.clone();
                        p.provider = original_provider;
                        id.clone()
                    };
                    profile_id_migration_targets.insert(id.clone(), final_id.clone());
                    profile_id_migration_targets
                        .insert(normalize_profile_id_provider(&id), final_id.clone());
                    new_profiles.insert(final_id.clone(), ap);
                    new_persisted_profiles.insert(final_id, p);
                }
            } else {
                // Neither resolved nor dropped: carried untouched because its
                // secrets are only in the unavailable keychain.
                new_persisted_profiles.insert(id, p);
            }
        }
        for profile_id in persisted.active_profiles.values_mut() {
            let normalized = normalize_profile_id_provider(profile_id);
            if let Some(target) = profile_id_migration_targets.get(&normalized) {
                if profile_id != target {
                    *profile_id = target.clone();
                }
            }
        }
        if profile_casing_changed_count > 0 {
            if profile_migration_conflicts > 0 {
                log::warn!(
                    "[auth] profile migration: {profile_migration_conflicts} case-variant \
                     collision(s) resolved by preferring the existing lowercase entry"
                );
            }
            log::debug!(
                "[auth] profile migration: normalized {profile_casing_changed_count} profile id(s) to lowercase"
            );
        }
        persisted.profiles = new_persisted_profiles;
        profiles = new_profiles;
        if !pending_keychain_deletes.is_empty() {
            log::debug!(
                "[auth] deferred {} old keyring entries for profile migration",
                pending_keychain_deletes.len()
            );
        }
        Ok(AuthProfilesData {
            schema_version: persisted.schema_version,
            updated_at: parse_datetime_with_fallback(&persisted.updated_at),
            active_profiles: persisted.active_profiles,
            profiles,
        })
    }
}

/// An OAuth entry whose secrets are only in the OS keychain: its JSON holds
/// none of them. While the keychain is unavailable it cannot be resolved, so
/// it is carried untouched (neither handed out nor purged) until it reads again.
pub(super) fn keychain_held_oauth(p: &PersistedAuthProfile) -> bool {
    p.access_token.is_none()
        && p.refresh_token.is_none()
        && p.id_token.is_none()
        && p.token.is_none()
        && matches!(parse_profile_kind(&p.kind), Ok(AuthProfileKind::OAuth))
}

pub(super) fn preserve_keyring_only_oauth_after_access_failure(
    profile: &PersistedAuthProfile,
    access_failed: bool,
) -> bool {
    access_failed && keychain_held_oauth(profile)
}
