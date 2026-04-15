use super::profiles::{
    profile_id, AuthProfile, AuthProfileKind, AuthProfilesData, AuthProfilesStore,
};
use crate::openhuman::config::Config;
use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Provider id for the in-app session token profile (matches desktop/web handoff).
pub const APP_SESSION_PROVIDER: &str = "app-session";
/// Default named profile when none is specified.
pub const DEFAULT_AUTH_PROFILE_NAME: &str = "default";

const DEFAULT_PROFILE_NAME: &str = "default";

#[derive(Clone)]
pub struct AuthService {
    store: AuthProfilesStore,
}

impl AuthService {
    pub fn from_config(config: &Config) -> Self {
        let state_dir = state_dir_from_config(config);
        Self::new(&state_dir, config.secrets.encrypt)
    }

    pub fn new(state_dir: &Path, encrypt_secrets: bool) -> Self {
        Self {
            store: AuthProfilesStore::new(state_dir, encrypt_secrets),
        }
    }

    pub fn load_profiles(&self) -> Result<AuthProfilesData> {
        self.store.load()
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
}

pub fn normalize_provider(provider: &str) -> Result<String> {
    let normalized = provider.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        anyhow::bail!("Provider name cannot be empty");
    }
    Ok(normalized)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::credentials::profiles::{AuthProfile, AuthProfileKind};

    #[test]
    fn normalize_provider_basic() {
        assert_eq!(normalize_provider("OpenAI").unwrap(), "openai");
    }

    #[test]
    fn normalize_provider_trims_whitespace_and_lowercases() {
        assert_eq!(normalize_provider("  GitHub  ").unwrap(), "github");
        assert_eq!(normalize_provider("OPENAI-CODEX").unwrap(), "openai-codex");
    }

    #[test]
    fn normalize_provider_rejects_empty_and_whitespace_only() {
        assert!(normalize_provider("").is_err());
        assert!(normalize_provider("   ").is_err());
        assert!(normalize_provider("\t\n").is_err());
    }

    #[test]
    fn default_profile_id_uses_default_name() {
        // Must line up with the `DEFAULT_PROFILE_NAME` constant so
        // callers that expect "<provider>:default" keep working.
        assert_eq!(default_profile_id("openai"), "openai:default");
        assert_eq!(default_profile_id("anthropic"), "anthropic:default");
    }

    #[test]
    fn resolve_requested_profile_id_passes_through_fully_qualified_ids() {
        assert_eq!(
            resolve_requested_profile_id("openai", "openai:work"),
            "openai:work"
        );
        // Even a mismatched-provider qualified id is preserved verbatim —
        // the caller is responsible for validation downstream.
        assert_eq!(
            resolve_requested_profile_id("openai", "github:personal"),
            "github:personal"
        );
    }

    #[test]
    fn resolve_requested_profile_id_prefixes_bare_names() {
        assert_eq!(
            resolve_requested_profile_id("openai", "work"),
            "openai:work"
        );
        assert_eq!(
            resolve_requested_profile_id("openai", "default"),
            "openai:default"
        );
    }

    #[test]
    fn state_dir_from_config_uses_config_path_parent() {
        let mut config = Config::default();
        config.config_path = PathBuf::from("/tmp/openhuman-test/config.toml");
        assert_eq!(
            state_dir_from_config(&config),
            PathBuf::from("/tmp/openhuman-test")
        );
    }

    #[test]
    fn state_dir_from_config_falls_back_to_dot_when_no_parent() {
        let mut config = Config::default();
        // A bare filename has no parent component (empty string) — we
        // treat that as cwd.
        config.config_path = PathBuf::from("");
        // Empty PathBuf has no parent at all → fallback ".".
        let dir = state_dir_from_config(&config);
        // Either "." (our fallback) or "" (parent of a path with just a
        // filename) is acceptable — both behave as cwd.
        assert!(dir == PathBuf::from(".") || dir.as_os_str().is_empty());
    }

    #[test]
    fn select_profile_id_returns_none_when_override_not_found() {
        let data = AuthProfilesData::default();
        assert_eq!(select_profile_id(&data, "my-provider", Some("ghost")), None);
    }

    #[test]
    fn select_profile_id_returns_none_when_no_profiles_exist() {
        let data = AuthProfilesData::default();
        assert_eq!(select_profile_id(&data, "my-provider", None), None);
    }

    #[test]
    fn select_profile_id_falls_back_to_any_provider_profile() {
        // No active, no "default" — but there is a profile that belongs
        // to the provider. That profile should be returned.
        let mut data = AuthProfilesData::default();
        let id_work = profile_id("coolco", "work");
        data.profiles.insert(
            id_work.clone(),
            AuthProfile {
                id: id_work.clone(),
                provider: "coolco".into(),
                profile_name: "work".into(),
                kind: AuthProfileKind::Token,
                account_id: None,
                workspace_id: None,
                token_set: None,
                token: Some("t".into()),
                metadata: std::collections::BTreeMap::default(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
        );
        assert_eq!(select_profile_id(&data, "coolco", None), Some(id_work));
    }

    #[test]
    fn select_profile_id_override_with_colon_is_used_verbatim() {
        let mut data = AuthProfilesData::default();
        let exotic_id = "openai:very-custom".to_string();
        data.profiles.insert(
            exotic_id.clone(),
            AuthProfile {
                id: exotic_id.clone(),
                provider: "openai".into(),
                profile_name: "very-custom".into(),
                kind: AuthProfileKind::Token,
                account_id: None,
                workspace_id: None,
                token_set: None,
                token: Some("t".into()),
                metadata: std::collections::BTreeMap::default(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
        );
        assert_eq!(
            select_profile_id(&data, "openai", Some("openai:very-custom")),
            Some(exotic_id)
        );
    }

    #[test]
    fn select_profile_prefers_override_then_active_then_default() {
        let mut data = AuthProfilesData::default();
        let id_active = profile_id("my-provider", "work");
        let id_default = profile_id("my-provider", "default");

        data.profiles.insert(
            id_default.clone(),
            AuthProfile {
                id: id_default.clone(),
                provider: "my-provider".into(),
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
                provider: "my-provider".into(),
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
            .insert("my-provider".into(), id_active.clone());

        assert_eq!(
            select_profile_id(&data, "my-provider", Some("default")),
            Some(id_default.clone())
        );
        assert_eq!(
            select_profile_id(&data, "my-provider", None),
            Some(id_active.clone())
        );
        data.active_profiles.clear();
        assert_eq!(
            select_profile_id(&data, "my-provider", None),
            Some(id_default)
        );
    }
}
