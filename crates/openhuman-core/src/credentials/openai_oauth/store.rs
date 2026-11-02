//! Persist and resolve OpenAI OAuth tokens for the `openai` cloud provider slug.

use std::sync::LazyLock;
use tokio::sync::Mutex;

use chrono::{Duration, Utc};
use tinyinference_providers::oauth::{openai_account_id_from_access_token, OAuthTokenSet};

use crate::config::Config;
use crate::credentials::profiles::{AuthProfile, AuthProfilesStore, TokenSet};
use crate::credentials::{state_dir_from_config, AuthService, ProviderOAuthTokens};

const LOG_PREFIX: &str = "[inference][openai-oauth][store]";

/// Serializes lookup/refresh/persist for the single OpenAI OAuth profile.
/// The on-disk profile lock protects individual writes, while this lock keeps
/// a rotating refresh token from being used by two concurrent lookups.
static OPENAI_OAUTH_PROFILE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(Mutex::default);

pub const OPENAI_PROVIDER_KEY: &str = "provider:openai";
pub const OPENAI_OAUTH_PROFILE_NAME: &str = "oauth";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiOAuthCredentials {
    pub access_token: String,
    pub account_id: Option<String>,
}

fn token_set_from_oauth(token: &OAuthTokenSet) -> TokenSet {
    let expires_at =
        (token.expires_in > 0).then(|| Utc::now() + Duration::seconds(token.expires_in as i64));
    TokenSet {
        access_token: token.access_token.clone(),
        refresh_token: token
            .refresh_token
            .clone()
            .filter(|refresh| !refresh.is_empty()),
        id_token: token.id_token.clone(),
        expires_at,
        token_type: Some("Bearer".to_string()),
        scope: None,
    }
}

fn normalize_persisted_token_set(mut token_set: TokenSet) -> Result<TokenSet, String> {
    let access_token = token_set.access_token.trim().to_string();
    if access_token.is_empty() {
        return Err("OpenAI OAuth token is missing access_token".to_string());
    }
    token_set.access_token = access_token;
    token_set.refresh_token = token_set.refresh_token.and_then(|value| {
        let trimmed = value.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    });
    token_set.id_token = token_set.id_token.and_then(|value| {
        let trimmed = value.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    });
    token_set.token_type = token_set.token_type.and_then(|value| {
        let trimmed = value.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    });
    token_set.scope = token_set.scope.and_then(|value| {
        let trimmed = value.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    });
    Ok(token_set)
}

pub async fn persist_openai_oauth_token(
    config: &Config,
    token: &OAuthTokenSet,
) -> Result<AuthProfile, String> {
    let account_id = openai_account_id_from_access_token(token.access_token.trim());
    persist_openai_oauth_tokens(
        config,
        &ProviderOAuthTokens {
            token: token.clone(),
            account_id,
        },
    )
    .await
}

pub async fn persist_openai_oauth_tokens(
    config: &Config,
    tokens: &ProviderOAuthTokens,
) -> Result<AuthProfile, String> {
    let _profile_lock = OPENAI_OAUTH_PROFILE_LOCK.lock().await;
    persist_openai_oauth_token_set(
        config,
        token_set_from_oauth(&tokens.token),
        tokens.account_id.clone(),
    )
    .await
}

pub(super) async fn persist_openai_oauth_token_set(
    config: &Config,
    token_set: TokenSet,
    account_id: Option<String>,
) -> Result<AuthProfile, String> {
    let token_set = normalize_persisted_token_set(token_set)?;
    let mut profile =
        AuthProfile::new_oauth(OPENAI_PROVIDER_KEY, OPENAI_OAUTH_PROFILE_NAME, token_set);
    if let Some(account_id) = account_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        profile
            .metadata
            .insert("account_id".to_string(), account_id);
    }

    let store = auth_profiles_store(config);
    store
        .upsert_profile(profile.clone(), true)
        .await
        .map_err(|e| e.to_string())?;
    Ok(profile)
}

fn auth_profiles_store(config: &Config) -> AuthProfilesStore {
    AuthProfilesStore::new_with_config(
        &state_dir_from_config(config),
        config.secrets.encrypt,
        config,
    )
}

/// Look up the OpenAI bearer token sourced from the OAuth (ChatGPT
/// subscription) flow. Returns `Ok(None)` when no OAuth profile is present or
/// when the access token is empty. API-key fallback for the `openai` slug is
/// handled by the standard `lookup_key_for_slug` path — this function is
/// OAuth-only so the standard path's env/audit/metrics logic still runs.
pub async fn lookup_openai_bearer_token(config: &Config) -> Result<Option<String>, String> {
    Ok(lookup_openai_oauth_credentials(config)
        .await?
        .map(|credentials| credentials.access_token))
}

pub async fn lookup_openai_oauth_credentials(
    config: &Config,
) -> Result<Option<OpenAiOAuthCredentials>, String> {
    let _profile_lock = OPENAI_OAUTH_PROFILE_LOCK.lock().await;
    let auth = AuthService::from_config(config);

    let profile = auth
        .get_profile(OPENAI_PROVIDER_KEY, Some(OPENAI_OAUTH_PROFILE_NAME))
        .await
        .map_err(|e| e.to_string())?;
    let Some(mut profile) = profile else {
        return Ok(None);
    };
    let Some(mut token_set) = profile.token_set.clone() else {
        return Ok(None);
    };

    if token_set.is_expiring_within(std::time::Duration::from_secs(120)) {
        if let Some(refresh) = token_set.refresh_token.clone() {
            let source = crate::credentials::provider_credential_source().ok_or_else(|| {
                "provider OAuth refresh is unavailable: no host credential source installed"
                    .to_string()
            })?;
            if let Some(fresh) =
                source.resolve_openai_oauth(OPENAI_PROVIDER_KEY, &refresh, token_set.expires_at)?
            {
                let mut refreshed = token_set_from_oauth(&fresh);
                // Providers do not always repeat stable fields in a refresh
                // response. Preserve them from the stored profile, while
                // accepting a newly rotated refresh token when supplied.
                if refreshed.refresh_token.is_none() {
                    refreshed.refresh_token = token_set.refresh_token.clone();
                }
                if refreshed.id_token.is_none() {
                    refreshed.id_token = token_set.id_token.clone();
                }
                refreshed.scope = token_set.scope.clone();
                token_set = refreshed;
                profile.token_set = Some(token_set.clone());
                if let Err(e) = auth_profiles_store(config)
                    .upsert_profile(profile.clone(), true)
                    .await
                {
                    log::warn!(
                        "{LOG_PREFIX} failed to persist refreshed token: {e}; \
                         fresh access token will be lost on restart"
                    );
                }
            }
        }
    }

    let access = token_set.access_token.trim();
    if access.is_empty() {
        Ok(None)
    } else {
        Ok(Some(OpenAiOAuthCredentials {
            access_token: access.to_string(),
            account_id: profile
                .metadata
                .get("account_id")
                .map(String::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .or_else(|| openai_account_id_from_access_token(access)),
        }))
    }
}
