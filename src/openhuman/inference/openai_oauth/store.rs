//! Persist and resolve OpenAI OAuth tokens for the `openai` cloud provider slug.

use std::path::{Path, PathBuf};

use base64::Engine;
use chrono::{Duration, TimeZone, Utc};
use motosan_ai_oauth::Token;
use serde::Deserialize;

use crate::openhuman::config::Config;
use crate::openhuman::credentials::profiles::{AuthProfile, AuthProfilesStore, TokenSet};
use crate::openhuman::credentials::{state_dir_from_config, AuthService};

use super::config::codex_oauth_config;

const LOG_PREFIX: &str = "[inference][openai-oauth][store]";

pub const OPENAI_PROVIDER_KEY: &str = "provider:openai";
pub const OPENAI_OAUTH_PROFILE_NAME: &str = "oauth";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiOAuthCredentials {
    pub access_token: String,
    pub account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CodexCliAuthFile {
    #[serde(default)]
    tokens: Option<CodexCliTokens>,
}

#[derive(Debug, Deserialize)]
struct CodexCliTokens {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    account_id: Option<String>,
}

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
    let account_id = extract_account_id_from_access_token(&token.access_token);
    persist_openai_oauth_token_set(config, token_set_from_codex(token), account_id)
}

pub(super) fn persist_openai_oauth_token_set(
    config: &Config,
    token_set: TokenSet,
    account_id: Option<String>,
) -> Result<AuthProfile, String> {
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
    let parsed: CodexCliAuthFile = serde_json::from_slice(&bytes).map_err(|e| {
        log::warn!(
            "{LOG_PREFIX} codex_cli_import:parse_failed path={} error={e}",
            path.display()
        );
        format!(
            "Could not parse Codex CLI auth at {}: {e}. Run `codex login` again, then try Codex auth again.",
            path.display()
        )
    })?;
    let tokens = parsed.tokens.ok_or_else(|| {
        log::warn!(
            "{LOG_PREFIX} codex_cli_import:missing_tokens path={}",
            path.display()
        );
        format!(
            "Codex CLI auth at {} has no tokens. Run `codex login` first, then try Codex auth again.",
            path.display()
        )
    })?;

    let access_token = tokens.access_token.unwrap_or_default().trim().to_string();
    if access_token.is_empty() {
        log::warn!(
            "{LOG_PREFIX} codex_cli_import:missing_access_token path={}",
            path.display()
        );
        return Err(format!(
            "Codex CLI auth at {} has no access token. Run `codex login` first, then try Codex auth again.",
            path.display()
        ));
    }

    let refresh_token = tokens
        .refresh_token
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let id_token = tokens
        .id_token
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let account_id = tokens
        .account_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| extract_account_id_from_access_token(&access_token));

    log::info!(
        "{LOG_PREFIX} codex_cli_import:persist_start path={} account_id_present={}",
        path.display(),
        account_id.is_some()
    );
    let profile = persist_openai_oauth_token_set(
        config,
        TokenSet {
            access_token: access_token.clone(),
            refresh_token,
            id_token,
            expires_at: extract_expires_at_from_access_token(&access_token),
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
    let json = decode_access_token_payload(access_token)?;
    json.get("https://api.openai.com/auth")
        .and_then(|v| v.get("chatgpt_account_id"))
        .or_else(|| json.get("chatgpt_account_id"))
        .or_else(|| json.get("sub"))
        .or_else(|| json.get("account_id"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn extract_expires_at_from_access_token(access_token: &str) -> Option<chrono::DateTime<Utc>> {
    let json = decode_access_token_payload(access_token)?;
    let exp = json
        .get("exp")
        .and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse::<i64>().ok()))?;
    Utc.timestamp_opt(exp, 0).single()
}

fn decode_access_token_payload(access_token: &str) -> Option<serde_json::Value> {
    let payload = access_token.split('.').nth(1)?;
    let padded = match payload.len() % 4 {
        0 => payload.to_string(),
        n => format!("{}{}", payload, "=".repeat(4 - n)),
    };
    let bytes = base64::engine::general_purpose::URL_SAFE
        .decode(padded.as_bytes())
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.as_bytes()))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
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
                    token_set = token_set_from_codex(&fresh);
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
                .or_else(|| extract_account_id_from_access_token(access)),
        }))
    }
}
