//! Persist and resolve OpenAI OAuth tokens for the `openai` cloud provider slug.

use std::path::{Path, PathBuf};

use chrono::{Duration, TimeZone, Utc};
use tinyinference_providers::oauth::{
    openai_account_id_from_access_token, openai_codex_config, parse_openai_codex_auth_json,
    refresh_access_token, OAuthTokenSet,
};

use crate::config::Config;
use crate::security::credentials::profiles::{AuthProfile, AuthProfilesStore, TokenSet};
use crate::security::credentials::{state_dir_from_config, AuthService};

const LOG_PREFIX: &str = "[inference][openai-oauth][store]";

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

pub fn persist_openai_oauth_token(
    config: &Config,
    token: &OAuthTokenSet,
) -> Result<AuthProfile, String> {
    let account_id = openai_account_id_from_access_token(token.access_token.trim());
    persist_openai_oauth_token_set(config, token_set_from_oauth(token), account_id)
}

pub(super) fn persist_openai_oauth_token_set(
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
        .map_err(|e| e.to_string())?;
    Ok(profile)
}

fn codex_cli_auth_path() -> Result<PathBuf, String> {
    if let Some(codex_home) = std::env::var_os("CODEX_HOME") {
        let path = PathBuf::from(codex_home);
        if !path.as_os_str().is_empty() {
            return Ok(path.join("auth.json"));
        }
    }

    let home = home_dir_from_env()
        .ok_or_else(|| "home directory is not set; cannot find ~/.codex/auth.json".to_string())?;
    Ok(home.join(".codex").join("auth.json"))
}

fn home_dir_from_env() -> Option<PathBuf> {
    for key in ["HOME", "USERPROFILE"] {
        if let Some(value) = std::env::var_os(key) {
            let path = PathBuf::from(value);
            if !path.as_os_str().is_empty() {
                return Some(path);
            }
        }
    }

    match (std::env::var_os("HOMEDRIVE"), std::env::var_os("HOMEPATH")) {
        (Some(drive), Some(path))
            if !drive.as_os_str().is_empty() && !path.as_os_str().is_empty() =>
        {
            Some(PathBuf::from(drive).join(path))
        }
        _ => None,
    }
}

pub(super) fn import_codex_cli_auth_from_path(
    config: &Config,
    path: &Path,
) -> Result<AuthProfile, String> {
    log::info!(
        "{LOG_PREFIX} codex_cli_import:start path={}",
        path.display()
    );
    let bytes = std::fs::read(path).map_err(|e| {
        log::warn!(
            "{LOG_PREFIX} codex_cli_import:read_failed path={} error={e}",
            path.display()
        );
        format!(
            "Could not read Codex CLI auth at {}: {e}. Run `codex login` first, then try Codex auth again.",
            path.display()
        )
    })?;
    let imported = parse_openai_codex_auth_json(&bytes).map_err(|e| {
        log::warn!(
            "{LOG_PREFIX} codex_cli_import:parse_failed path={} error={e}",
            path.display()
        );
        format!(
            "Could not parse Codex CLI auth at {}: {e}. Run `codex login` again, then try Codex auth again.",
            path.display()
        )
    })?;
    let account_id = imported.account_id;

    log::info!(
        "{LOG_PREFIX} codex_cli_import:persist_start path={} account_id_present={}",
        path.display(),
        account_id.is_some()
    );
    let profile = persist_openai_oauth_token_set(
        config,
        TokenSet {
            access_token: imported.token.access_token,
            refresh_token: imported.token.refresh_token,
            id_token: imported.token.id_token,
            expires_at: imported
                .expires_at_unix
                .and_then(|timestamp| Utc.timestamp_opt(timestamp, 0).single()),
            token_type: Some("Bearer".to_string()),
            scope: None,
        },
        account_id,
    )?;
    log::info!(
        "{LOG_PREFIX} codex_cli_import:ok path={} profile_id={}",
        path.display(),
        profile.id
    );
    Ok(profile)
}

pub fn import_codex_cli_auth(config: &Config) -> Result<AuthProfile, String> {
    let path = codex_cli_auth_path()?;
    import_codex_cli_auth_from_path(config, &path)
}

fn auth_profiles_store(config: &Config) -> AuthProfilesStore {
    AuthProfilesStore::new(&state_dir_from_config(config), config.secrets.encrypt)
}

fn try_refresh_oauth_token(refresh: &str) -> Result<OAuthTokenSet, String> {
    let cfg = openai_codex_config("http://127.0.0.1:1455/auth/callback");
    #[cfg(test)]
    let cfg = if let Some(token_url) = local_refresh_url_override() {
        let mut cfg = cfg;
        cfg.token_url = token_url;
        cfg
    } else {
        cfg
    };
    let refresh = refresh.to_string();
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        // `block_in_place` lets the multi-thread runtime move other tasks off this
        // worker before we synchronously drive the refresh future, avoiding a
        // deadlock when this lookup is reached from inside an async caller.
        let refreshed =
            tokio::task::block_in_place(|| handle.block_on(refresh_access_token(&cfg, &refresh)))
                .map_err(|e| e.to_string())?;
        if refreshed.access_token.trim().is_empty() {
            return Err("OAuth refresh response contained an empty access token".to_owned());
        }
        return Ok(refreshed);
    }
    Err("tokio runtime required to refresh openai oauth token".to_string())
}

/// Local-only endpoint override for OAuth refresh tests. Production builds do
/// not read this variable, and tests cannot redirect refresh credentials away
/// from loopback.
#[cfg(test)]
fn local_refresh_url_override() -> Option<String> {
    let endpoint = std::env::var("OPENAI_CODEX_OAUTH_TOKEN_URL").ok()?;
    let parsed = reqwest::Url::parse(&endpoint).ok()?;
    let host = parsed.host_str()?;
    (parsed.scheme() == "http" && matches!(host, "localhost" | "127.0.0.1" | "::1"))
        .then(|| endpoint.clone())
}

/// Look up the OpenAI bearer token sourced from the OAuth (ChatGPT
/// subscription) flow. Returns `Ok(None)` when no OAuth profile is present or
/// when the access token is empty. API-key fallback for the `openai` slug is
/// handled by the standard `lookup_key_for_slug` path — this function is
/// OAuth-only so the standard path's env/audit/metrics logic still runs.
pub fn lookup_openai_bearer_token(config: &Config) -> Result<Option<String>, String> {
    Ok(lookup_openai_oauth_credentials(config)?.map(|credentials| credentials.access_token))
}

pub fn lookup_openai_oauth_credentials(
    config: &Config,
) -> Result<Option<OpenAiOAuthCredentials>, String> {
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
                    token_set = token_set_from_oauth(&fresh);
                    profile.token_set = Some(token_set.clone());
                    if let Err(e) =
                        auth_profiles_store(config).upsert_profile(profile.clone(), true)
                    {
                        log::warn!(
                            "{LOG_PREFIX} failed to persist refreshed token: {e}; \
                             fresh access token will be lost on restart"
                        );
                    }
                }
                Err(e) => {
                    log::warn!("{LOG_PREFIX} oauth refresh failed: {e}");
                    // If the token has already passed its expiry there is no
                    // point proceeding — the next inference call will hit 401
                    // and the user sees a generic error with no remedy.
                    // Return a string that `is_openai_oauth_session_expired_message`
                    // recognises ("authentication token is expired") so
                    // `classify_inference_error` routes to the Codex reconnect
                    // message (Settings → Integrations), not the OpenHuman
                    // app-session sign-in flow. (#5869)
                    if token_set.is_expiring_within(std::time::Duration::ZERO) {
                        return Err(format!(
                            "Codex authentication token is expired — refresh failed: {e}. \
                             Please reconnect Codex in Settings → Integrations."
                        ));
                    }
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
