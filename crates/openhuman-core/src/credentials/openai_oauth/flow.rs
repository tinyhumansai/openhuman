//! OpenHuman credential-store bindings for the TinyInference OAuth flow.

use crate::config::Config;
use chrono::{DateTime, Utc};
use serde::Serialize;

use super::store::{persist_openai_oauth_tokens, OPENAI_OAUTH_PROFILE_NAME, OPENAI_PROVIDER_KEY};

const LOG_PREFIX: &str = "[inference][openai-oauth]";
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

pub async fn start_openai_oauth(config: &Config) -> Result<OpenAiOAuthStartResult, String> {
    let source = crate::credentials::provider_credential_source().ok_or_else(|| {
        "provider OAuth is unavailable: no host credential source installed".to_string()
    })?;
    let start = source.start_openai_oauth(config).await?;
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
    let source = crate::credentials::provider_credential_source().ok_or_else(|| {
        "provider OAuth is unavailable: no host credential source installed".to_string()
    })?;
    let tokens = source.complete_openai_oauth(config, callback_input).await?;
    let profile = persist_openai_oauth_tokens(config, &tokens).await?;
    log::info!("{LOG_PREFIX} oauth complete profile_id={}", profile.id);
    Ok(serde_json::json!({
        "connected": true,
        "profileId": profile.id,
        "provider": OPENAI_PROVIDER_KEY,
        "authMethod": "oauth",
    }))
}

pub async fn import_openai_oauth_from_codex_cli(
    config: &Config,
) -> Result<serde_json::Value, String> {
    let source = crate::credentials::provider_credential_source().ok_or_else(|| {
        "provider OAuth is unavailable: no host credential source installed".to_string()
    })?;
    let tokens = source.import_openai_oauth_from_codex_cli(config)?;
    let profile = persist_openai_oauth_tokens(config, &tokens).await?;
    Ok(serde_json::json!({
        "connected": true,
        "profileId": profile.id,
        "provider": OPENAI_PROVIDER_KEY,
        "authMethod": "oauth",
        "source": "codex_cli",
    }))
}

pub async fn openai_oauth_status(config: &Config) -> Result<OpenAiOAuthStatusResult, String> {
    use crate::credentials::profiles::AuthProfileKind;
    use crate::credentials::AuthService;

    let profile = AuthService::from_config(config)
        .get_profile(OPENAI_PROVIDER_KEY, Some(OPENAI_OAUTH_PROFILE_NAME))
        .await
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
    use crate::credentials::AuthService;

    // Clear host-owned pending state before removing the durable profile. If
    // the secret backend is unavailable, the connected profile remains intact.
    let source = crate::credentials::provider_credential_source().ok_or_else(|| {
        "provider OAuth is unavailable: no host credential source installed".to_string()
    })?;
    source.clear_openai_oauth_pending(config).await?;
    let removed = AuthService::from_config(config)
        .remove_profile(OPENAI_PROVIDER_KEY, OPENAI_OAUTH_PROFILE_NAME)
        .await
        .map_err(|error| error.to_string())?;
    Ok(serde_json::json!({ "disconnected": removed }))
}
