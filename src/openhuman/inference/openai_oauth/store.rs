//! Persist and resolve OpenAI OAuth tokens for the `openai` cloud provider slug.

use base64::Engine;
use chrono::{Duration, Utc};
use motosan_ai_oauth::Token;

use crate::openhuman::config::Config;
use crate::openhuman::credentials::profiles::{AuthProfile, AuthProfilesStore, TokenSet};
use crate::openhuman::credentials::{state_dir_from_config, AuthService};

use super::config::codex_oauth_config;

const LOG_PREFIX: &str = "[inference][openai-oauth][store]";

pub const OPENAI_PROVIDER_KEY: &str = "provider:openai";
pub const OPENAI_OAUTH_PROFILE_NAME: &str = "oauth";

fn token_set_from_codex(token: &Token) -> TokenSet {
    let expires_at =
        (token.expires_in > 0).then(|| Utc::now() + Duration::seconds(token.expires_in as i64));
    TokenSet {
        access_token: token.access_token.clone(),
        refresh_token: (!token.refresh_token.is_empty()).then(|| token.refresh_token.clone()),
        id_token: token.id_token.clone(),
        expires_at,
        token_type: Some("Bearer".to_string()),
        scope: None,
    }
}

pub fn persist_openai_oauth_token(config: &Config, token: &Token) -> Result<AuthProfile, String> {
    let mut profile = AuthProfile::new_oauth(
        OPENAI_PROVIDER_KEY,
        OPENAI_OAUTH_PROFILE_NAME,
        token_set_from_codex(token),
    );
    if let Some(account_id) = extract_account_id_from_access_token(&token.access_token) {
        profile
            .metadata
            .insert("account_id".to_string(), account_id);
    }

    let store = auth_profiles_store(config);
    store
        .upsert_profile(profile.clone(), true)
        .map_err(|e| e.to_string())?;
    Ok(profile)
}

fn auth_profiles_store(config: &Config) -> AuthProfilesStore {
    AuthProfilesStore::new(&state_dir_from_config(config), config.secrets.encrypt)
}

fn try_refresh_oauth_token(refresh: &str) -> Result<Token, String> {
    let cfg = codex_oauth_config();
    let refresh = refresh.to_string();
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        // `block_in_place` lets the multi-thread runtime move other tasks off this
        // worker before we synchronously drive the refresh future, avoiding a
        // deadlock when this lookup is reached from inside an async caller.
        return tokio::task::block_in_place(|| {
            handle.block_on(motosan_ai_oauth::refresh(&cfg, &refresh))
        })
        .map_err(|e| e.to_string());
    }
    Err("tokio runtime required to refresh openai oauth token".to_string())
}

fn extract_account_id_from_access_token(access_token: &str) -> Option<String> {
    let payload = access_token.split('.').nth(1)?;
    let padded = match payload.len() % 4 {
        0 => payload.to_string(),
        n => format!("{}{}", payload, "=".repeat(4 - n)),
    };
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(padded.as_bytes())
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(padded.as_bytes()))
        .ok()?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    json.get("sub")
        .or_else(|| json.get("account_id"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// Look up the OpenAI bearer token sourced from the OAuth (ChatGPT
/// subscription) flow. Returns `Ok(None)` when no OAuth profile is present or
/// when the access token is empty. API-key fallback for the `openai` slug is
/// handled by the standard `lookup_key_for_slug` path — this function is
/// OAuth-only so the standard path's env/audit/metrics logic still runs.
pub fn lookup_openai_bearer_token(config: &Config) -> Result<Option<String>, String> {
    let auth = AuthService::from_config(config);

    let profile = auth
        .get_profile(OPENAI_PROVIDER_KEY, Some(OPENAI_OAUTH_PROFILE_NAME))
        .map_err(|e| e.to_string())?;
    let Some(mut profile) = profile else {
        return Ok(None);
    };
    let Some(mut token_set) = profile.token_set.clone() else {
        return Ok(None);
    };

    let skew = Duration::minutes(2);
    if token_set.is_expiring_within(std::time::Duration::from_secs(
        skew.num_seconds().unsigned_abs(),
    )) {
        if let Some(refresh) = token_set.refresh_token.clone() {
            match try_refresh_oauth_token(&refresh) {
                Ok(fresh) => {
                    token_set = token_set_from_codex(&fresh);
                    profile.token_set = Some(token_set.clone());
                    if let Err(e) = auth_profiles_store(config).upsert_profile(profile, true) {
                        log::warn!(
                            "{LOG_PREFIX} failed to persist refreshed token: {e}; \
                             fresh access token will be lost on restart"
                        );
                    }
                }
                Err(e) => {
                    log::warn!("{LOG_PREFIX} oauth refresh failed: {e}");
                }
            }
        }
    }

    let access = token_set.access_token.trim();
    if access.is_empty() {
        Ok(None)
    } else {
        Ok(Some(access.to_string()))
    }
}
