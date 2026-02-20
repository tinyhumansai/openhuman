pub mod anthropic_token;
pub mod openai_oauth;
pub mod profiles;

use crate::auth::openai_oauth::refresh_access_token;
use crate::auth::profiles::{
    profile_id, AuthProfile, AuthProfileKind, AuthProfilesData, AuthProfilesStore,
};
use crate::alphahuman::config::Config;
use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const OPENAI_CODEX_PROVIDER: &str = "openai-codex";
const ANTHROPIC_PROVIDER: &str = "anthropic";
const DEFAULT_PROFILE_NAME: &str = "default";
const OPENAI_REFRESH_SKEW_SECS: u64 = 90;
const OPENAI_REFRESH_FAILURE_BACKOFF_SECS: u64 = 10;
static REFRESH_BACKOFFS: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();

#[derive(Clone)]
pub struct AuthService {
    store: AuthProfilesStore,
    client: reqwest::Client,
}

impl AuthService {
    pub fn from_config(config: &Config) -> Self {
        let state_dir = state_dir_from_config(config);
        Self::new(&state_dir, config.secrets.encrypt)
    }

    pub fn new(state_dir: &Path, encrypt_secrets: bool) -> Self {
        Self {
            store: AuthProfilesStore::new(state_dir, encrypt_secrets),
            client: reqwest::Client::new(),
        }
    }

    pub fn load_profiles(&self) -> Result<AuthProfilesData> {
        self.store.load()
    }

    pub fn store_openai_tokens(
        &self,
        profile_name: &str,
        token_set: crate::auth::profiles::TokenSet,
        account_id: Option<String>,
        set_active: bool,
    ) -> Result<AuthProfile> {
        let mut profile = AuthProfile::new_oauth(OPENAI_CODEX_PROVIDER, profile_name, token_set);
        profile.account_id = account_id;
        self.store.upsert_profile(profile.clone(), set_active)?;
        Ok(profile)
    }

    pub fn store_provider_token(
        &self,
        provider: &str,
        profile_name: &str,
        token: &str,
        metadata: HashMap<String, String>,
        set_active: bool,
    ) -> Result<AuthProfile> {
        let mut profile = AuthProfile::new_token(provider, profile_name, token.to_string());
        profile.metadata.extend(metadata);
        self.store.upsert_profile(profile.clone(), set_active)?;
        Ok(profile)
    }

    pub fn set_active_profile(&self, provider: &str, requested_profile: &str) -> Result<String> {
        let provider = normalize_provider(provider)?;
        let data = self.store.load()?;
        let profile_id = resolve_requested_profile_id(&provider, requested_profile);

        let profile = data
            .profiles
            .get(&profile_id)
            .ok_or_else(|| anyhow::anyhow!("Auth profile not found: {profile_id}"))?;

        if profile.provider != provider {
            anyhow::bail!(
                "Profile {profile_id} belongs to provider {}, not {}",
                profile.provider,
                provider
            );
        }

        self.store.set_active_profile(&provider, &profile_id)?;
        Ok(profile_id)
    }

    pub fn remove_profile(&self, provider: &str, requested_profile: &str) -> Result<bool> {
        let provider = normalize_provider(provider)?;
        let profile_id = resolve_requested_profile_id(&provider, requested_profile);
        self.store.remove_profile(&profile_id)
    }

    pub fn get_profile(
        &self,
        provider: &str,
        profile_override: Option<&str>,
    ) -> Result<Option<AuthProfile>> {
        let provider = normalize_provider(provider)?;
        let data = self.store.load()?;
        let Some(profile_id) = select_profile_id(&data, &provider, profile_override) else {
            return Ok(None);
        };
        Ok(data.profiles.get(&profile_id).cloned())
    }

    pub fn get_provider_bearer_token(
        &self,
        provider: &str,
        profile_override: Option<&str>,
    ) -> Result<Option<String>> {
        let profile = self.get_profile(provider, profile_override)?;
        let Some(profile) = profile else {
            return Ok(None);
        };

        let credential = match profile.kind {
            AuthProfileKind::Token => profile.token,
            AuthProfileKind::OAuth => profile.token_set.map(|t| t.access_token),
        };

        Ok(credential.filter(|t| !t.trim().is_empty()))
    }

    pub async fn get_valid_openai_access_token(
        &self,
        profile_override: Option<&str>,
    ) -> Result<Option<String>> {
        let data = tokio::task::spawn_blocking({
            let store = self.store.clone();
            move || store.load()
        })
        .await
        .map_err(|err| anyhow::anyhow!("Auth profile load task failed: {err}"))??;
        let Some(profile_id) = select_profile_id(&data, OPENAI_CODEX_PROVIDER, profile_override)
        else {
            return Ok(None);
        };

        let Some(profile) = data.profiles.get(&profile_id) else {
            return Ok(None);
        };

        let Some(token_set) = profile.token_set.as_ref() else {
            anyhow::bail!("OpenAI Codex auth profile is not OAuth-based: {profile_id}");
        };

        if !token_set.is_expiring_within(Duration::from_secs(OPENAI_REFRESH_SKEW_SECS)) {
            return Ok(Some(token_set.access_token.clone()));
        }

        let Some(refresh_token) = token_set.refresh_token.clone() else {
            return Ok(Some(token_set.access_token.clone()));
        };

        let refresh_lock = refresh_lock_for_profile(&profile_id);
        let _guard = refresh_lock.lock().await;

        // Re-load after waiting for lock to avoid duplicate refreshes.
        let data = tokio::task::spawn_blocking({
            let store = self.store.clone();
            move || store.load()
        })
        .await
        .map_err(|err| anyhow::anyhow!("Auth profile load task failed: {err}"))??;
        let Some(latest_profile) = data.profiles.get(&profile_id) else {
            return Ok(None);
        };

        let Some(latest_tokens) = latest_profile.token_set.as_ref() else {
            anyhow::bail!("OpenAI Codex auth profile is missing token set: {profile_id}");
        };

        if !latest_tokens.is_expiring_within(Duration::from_secs(OPENAI_REFRESH_SKEW_SECS)) {
            return Ok(Some(latest_tokens.access_token.clone()));
        }

        let refresh_token = latest_tokens.refresh_token.clone().unwrap_or(refresh_token);

        if let Some(remaining) = refresh_backoff_remaining(&profile_id) {
            anyhow::bail!(
                "OpenAI token refresh is in backoff for {remaining}s due to previous failures"
            );
        }

        let mut refreshed = match refresh_access_token(&self.client, &refresh_token).await {
            Ok(tokens) => {
                clear_refresh_backoff(&profile_id);
                tokens
            }
            Err(err) => {
                set_refresh_backoff(
                    &profile_id,
                    Duration::from_secs(OPENAI_REFRESH_FAILURE_BACKOFF_SECS),
                );
                return Err(err);
            }
        };
        if refreshed.refresh_token.is_none() {
            refreshed
                .refresh_token
                .clone_from(&latest_tokens.refresh_token);
        }

        let account_id = openai_oauth::extract_account_id_from_jwt(&refreshed.access_token)
            .or_else(|| latest_profile.account_id.clone());

        let updated = tokio::task::spawn_blocking({
            let store = self.store.clone();
            let profile_id = profile_id.clone();
            let refreshed = refreshed.clone();
            let account_id = account_id.clone();
            move || {
                store.update_profile(&profile_id, |profile| {
                    profile.kind = AuthProfileKind::OAuth;
                    profile.token_set = Some(refreshed.clone());
                    profile.account_id.clone_from(&account_id);
                    Ok(())
                })
            }
        })
        .await
        .map_err(|err| anyhow::anyhow!("Auth profile update task failed: {err}"))??;

        Ok(updated.token_set.map(|t| t.access_token))
    }
}

pub fn normalize_provider(provider: &str) -> Result<String> {
    let normalized = provider.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "openai-codex" | "openai_codex" | "codex" => Ok(OPENAI_CODEX_PROVIDER.to_string()),
        "anthropic" | "claude" | "claude-code" => Ok(ANTHROPIC_PROVIDER.to_string()),
        other if !other.is_empty() => Ok(other.to_string()),
        _ => anyhow::bail!("Provider name cannot be empty"),
    }
}

pub fn state_dir_from_config(config: &Config) -> PathBuf {
    config
        .config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}

pub fn default_profile_id(provider: &str) -> String {
    profile_id(provider, DEFAULT_PROFILE_NAME)
}

fn resolve_requested_profile_id(provider: &str, requested: &str) -> String {
    if requested.contains(':') {
        requested.to_string()
    } else {
        profile_id(provider, requested)
    }
}

pub fn select_profile_id(
    data: &AuthProfilesData,
    provider: &str,
    profile_override: Option<&str>,
) -> Option<String> {
    if let Some(override_profile) = profile_override {
        let requested = resolve_requested_profile_id(provider, override_profile);
        if data.profiles.contains_key(&requested) {
            return Some(requested);
        }
        return None;
    }

    if let Some(active) = data.active_profiles.get(provider) {
        if data.profiles.contains_key(active) {
            return Some(active.clone());
        }
    }

    let default = default_profile_id(provider);
    if data.profiles.contains_key(&default) {
        return Some(default);
    }

    data.profiles
        .iter()
        .find_map(|(id, profile)| (profile.provider == provider).then(|| id.clone()))
}

fn refresh_lock_for_profile(profile_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();

    let table = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = table.lock().expect("refresh lock table poisoned");

    guard
        .entry(profile_id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

fn refresh_backoff_remaining(profile_id: &str) -> Option<u64> {
    let map = REFRESH_BACKOFFS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().ok()?;
    let now = Instant::now();
    let deadline = guard.get(profile_id).copied()?;
    if deadline <= now {
        guard.remove(profile_id);
        return None;
    }
    Some((deadline - now).as_secs().max(1))
}

fn set_refresh_backoff(profile_id: &str, duration: Duration) {
    let map = REFRESH_BACKOFFS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut guard) = map.lock() {
        guard.insert(profile_id.to_string(), Instant::now() + duration);
    }
}

fn clear_refresh_backoff(profile_id: &str) {
    let map = REFRESH_BACKOFFS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut guard) = map.lock() {
        guard.remove(profile_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::profiles::{AuthProfile, AuthProfileKind};

    #[test]
    fn normalize_provider_aliases() {
        assert_eq!(normalize_provider("codex").unwrap(), "openai-codex");
        assert_eq!(normalize_provider("claude").unwrap(), "anthropic");
        assert_eq!(normalize_provider("openai").unwrap(), "openai");
    }

    #[test]
    fn select_profile_prefers_override_then_active_then_default() {
        let mut data = AuthProfilesData::default();
        let id_active = profile_id("openai-codex", "work");
        let id_default = profile_id("openai-codex", "default");

        data.profiles.insert(
            id_default.clone(),
            AuthProfile {
                id: id_default.clone(),
                provider: "openai-codex".into(),
                profile_name: "default".into(),
                kind: AuthProfileKind::Token,
                account_id: None,
                workspace_id: None,
                token_set: None,
                token: Some("x".into()),
                metadata: std::collections::BTreeMap::default(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
        );
        data.profiles.insert(
            id_active.clone(),
            AuthProfile {
                id: id_active.clone(),
                provider: "openai-codex".into(),
                profile_name: "work".into(),
                kind: AuthProfileKind::Token,
                account_id: None,
                workspace_id: None,
                token_set: None,
                token: Some("y".into()),
                metadata: std::collections::BTreeMap::default(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
        );

        data.active_profiles
            .insert("openai-codex".into(), id_active.clone());

        assert_eq!(
            select_profile_id(&data, "openai-codex", Some("default")),
            Some(id_default)
        );
        assert_eq!(
            select_profile_id(&data, "openai-codex", None),
            Some(id_active)
        );
    }
}
