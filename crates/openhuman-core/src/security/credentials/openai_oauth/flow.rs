//! OpenHuman credential-store bindings for the TinyInference OAuth flow.

use chrono::{DateTime, Utc};
use serde::Serialize;
use tinyinference_providers::oauth::{openai_codex_config, OAuthFlow};

use crate::config::Config;
use crate::security::credentials::state_dir_from_config;

use super::pending;
use super::store::{
    import_codex_cli_auth, persist_openai_oauth_token, OPENAI_OAUTH_PROFILE_NAME,
    OPENAI_PROVIDER_KEY,
};

const LOG_PREFIX: &str = "[inference][openai-oauth]";
const REDIRECT_URI: &str = "http://127.0.0.1:1455/auth/callback";

#[derive(Debug, Clone, Serialize)]
pub struct OpenAiOAuthStartResult {
    pub auth_url: String,
    pub state: String,
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAiOAuthStatusResult {
    pub connected: bool,
    pub profile_id: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub auth_method: Option<String>,
}

/// The flow over the pending-session store for this call, with any legacy
/// pending file adopted first.
async fn flow(config: &Config) -> Result<OAuthFlow, String> {
    let secrets = pending::store(config)?;
    pending::adopt_legacy(&state_dir_from_config(config), &secrets).await;
    Ok(OAuthFlow::new(openai_codex_config(REDIRECT_URI), secrets))
}

pub async fn start_openai_oauth(config: &Config) -> Result<OpenAiOAuthStartResult, String> {
    let start = flow(config).await?.start().await?;
    Ok(OpenAiOAuthStartResult {
        auth_url: start.auth_url,
        state: start.state,
        redirect_uri: start.redirect_uri,
    })
}

pub async fn complete_openai_oauth(
    config: &Config,
    callback_input: &str,
) -> Result<serde_json::Value, String> {
    let token = flow(config).await?.complete(callback_input).await?;
    let profile = persist_openai_oauth_token(config, &token)?;
    log::info!("{LOG_PREFIX} oauth complete profile_id={}", profile.id);
    Ok(serde_json::json!({
        "connected": true,
        "profileId": profile.id,
        "provider": OPENAI_PROVIDER_KEY,
        "authMethod": "oauth",
    }))
}

pub fn import_openai_oauth_from_codex_cli(config: &Config) -> Result<serde_json::Value, String> {
    let profile = import_codex_cli_auth(config)?;
    Ok(serde_json::json!({
        "connected": true,
        "profileId": profile.id,
        "provider": OPENAI_PROVIDER_KEY,
        "authMethod": "oauth",
        "source": "codex_cli",
    }))
}

pub fn openai_oauth_status(config: &Config) -> Result<OpenAiOAuthStatusResult, String> {
    use crate::security::credentials::profiles::AuthProfileKind;
    use crate::security::credentials::AuthService;

    let profile = AuthService::from_config(config)
        .get_profile(OPENAI_PROVIDER_KEY, Some(OPENAI_OAUTH_PROFILE_NAME))
        .map_err(|error| error.to_string())?;
    let Some(profile) = profile else {
        return Ok(OpenAiOAuthStatusResult {
            connected: false,
            profile_id: None,
            expires_at: None,
            auth_method: None,
        });
    };
    if profile.kind != AuthProfileKind::OAuth {
        return Ok(OpenAiOAuthStatusResult {
            connected: false,
            profile_id: Some(profile.id),
            expires_at: None,
            auth_method: Some("token".to_string()),
        });
    }
    Ok(OpenAiOAuthStatusResult {
        connected: true,
        profile_id: Some(profile.id),
        expires_at: profile
            .token_set
            .as_ref()
            .and_then(|token| token.expires_at),
        auth_method: Some("oauth".to_string()),
    })
}

pub async fn disconnect_openai_oauth(config: &Config) -> Result<serde_json::Value, String> {
    use crate::security::credentials::AuthService;

    let removed = AuthService::from_config(config)
        .remove_profile(OPENAI_PROVIDER_KEY, OPENAI_OAUTH_PROFILE_NAME)
        .map_err(|error| error.to_string())?;
    flow(config).await?.clear_pending().await;
    Ok(serde_json::json!({ "disconnected": removed }))
}
