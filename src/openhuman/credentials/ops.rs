//! JSON-RPC / CLI controller surface for credentials and app session auth.

use serde_json::json;

use crate::api::config::effective_api_url;
use crate::api::jwt::get_session_token;
use crate::api::rest::{user_id_from_profile_payload, BackendOAuthClient};
use crate::openhuman::config::Config;
use crate::openhuman::credentials::session_support::{
    build_session_state, parse_fields_value, profile_name_or_default, summarize_auth_profile,
};
use crate::openhuman::security::SecretStore;
use crate::rpc::RpcOutcome;

use super::{AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME};

fn secret_store_for_config(config: &Config) -> SecretStore {
    let data_dir = config
        .config_path
        .parent()
        .map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from);
    SecretStore::new(&data_dir, true)
}

pub async fn encrypt_secret(
    config: &Config,
    plaintext: &str,
) -> Result<RpcOutcome<String>, String> {
    let store = secret_store_for_config(config);
    let ciphertext = store.encrypt(plaintext).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(ciphertext, "secret encrypted"))
}

pub async fn decrypt_secret(
    config: &Config,
    ciphertext: &str,
) -> Result<RpcOutcome<String>, String> {
    let store = secret_store_for_config(config);
    let plaintext = store.decrypt(ciphertext).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(plaintext, "secret decrypted"))
}

pub async fn store_session(
    config: &Config,
    token: &str,
    user_id: Option<String>,
    user: Option<serde_json::Value>,
) -> Result<RpcOutcome<super::responses::AuthProfileSummary>, String> {
    let trimmed_token = token.trim();
    if trimmed_token.is_empty() {
        return Err("token is required".to_string());
    }

    let api_url = effective_api_url(&config.api_url);

    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let settings = client
        .fetch_current_user(trimmed_token)
        .await
        .map_err(|e| format!("Session validation failed (GET /auth/me): {e:#}"))?;

    let mut metadata = std::collections::HashMap::new();
    if let Some(uid) = user_id
        .and_then(|v| {
            let t = v.trim().to_string();
            (!t.is_empty()).then_some(t)
        })
        .or_else(|| user_id_from_profile_payload(&settings))
    {
        metadata.insert("user_id".to_string(), uid);
    }
    let user_for_store = user.unwrap_or(settings);
    metadata.insert("user_json".to_string(), user_for_store.to_string());

    let auth = AuthService::from_config(config);
    let profile = auth
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            trimmed_token,
            metadata,
            true,
        )
        .map_err(|e| e.to_string())?;

    Ok(RpcOutcome::new(
        summarize_auth_profile(&profile),
        vec![
            format!(
                "session JWT verified via GET /auth/me on {}",
                api_url.trim_end_matches('/')
            ),
            "session stored".to_string(),
        ],
    ))
}

pub async fn clear_session(config: &Config) -> Result<RpcOutcome<serde_json::Value>, String> {
    let auth = AuthService::from_config(config);
    let removed = auth
        .remove_profile(APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME)
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        json!({ "removed": removed }),
        "session cleared",
    ))
}

pub async fn auth_get_state(
    config: &Config,
) -> Result<RpcOutcome<super::responses::AuthStateResponse>, String> {
    let state = build_session_state(config)?;
    Ok(RpcOutcome::single_log(state, "session state fetched"))
}

pub async fn auth_get_session_token_json(
    config: &Config,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let token = get_session_token(config)?;
    Ok(RpcOutcome::single_log(
        json!({ "token": token }),
        "session token fetched",
    ))
}

pub async fn auth_get_me(config: &Config) -> Result<RpcOutcome<serde_json::Value>, String> {
    let api_url = effective_api_url(&config.api_url);
    let token = get_session_token(config)?.ok_or_else(|| "session JWT required".to_string())?;
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let user = client
        .fetch_current_user(&token)
        .await
        .map_err(|e| e.to_string())?;

    Ok(RpcOutcome::single_log(user, "current user fetched"))
}

pub async fn consume_login_token(
    config: &Config,
    login_token: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let token = login_token.trim();
    if token.is_empty() {
        return Err("loginToken is required".to_string());
    }

    let api_url = effective_api_url(&config.api_url);
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let jwt_token = client
        .consume_login_token(token)
        .await
        .map_err(|e| e.to_string())?;

    Ok(RpcOutcome::new(
        serde_json::json!({ "jwtToken": jwt_token }),
        vec![
            format!(
                "login token consumed via POST /telegram/login-tokens/:token/consume on {}",
                api_url.trim_end_matches('/')
            ),
            "session JWT received".to_string(),
        ],
    ))
}

pub async fn auth_create_channel_link_token(
    config: &Config,
    channel: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let channel = channel.trim();
    if channel.is_empty() {
        return Err("channel is required".to_string());
    }
    let channel = channel.to_lowercase();
    if !matches!(channel.as_str(), "telegram" | "discord") {
        return Err(format!("unsupported channel: {channel}"));
    }

    let api_url = effective_api_url(&config.api_url);
    let token = get_session_token(config)?.ok_or_else(|| "session JWT required".to_string())?;
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let payload = client
        .create_channel_link_token(&channel, &token)
        .await
        .map_err(|e| e.to_string())?;

    Ok(RpcOutcome::single_log(
        payload,
        "channel link token created",
    ))
}

pub async fn store_provider_credentials(
    config: &Config,
    provider: &str,
    profile: Option<&str>,
    token: Option<String>,
    fields: Option<serde_json::Value>,
    set_active: Option<bool>,
) -> Result<RpcOutcome<super::responses::AuthProfileSummary>, String> {
    let provider = provider.trim().to_string();
    if provider.is_empty() {
        return Err("provider is required".to_string());
    }

    let profile_name = profile_name_or_default(profile);
    let mut metadata = parse_fields_value(fields)?;
    let token = token
        .as_ref()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| metadata.get("token").cloned())
        .or_else(|| metadata.get("api_key").cloned())
        .unwrap_or_default();
    if token.is_empty() && metadata.is_empty() {
        return Err("provide at least one credential via token or fields".to_string());
    }
    metadata.remove("token");

    let auth = AuthService::from_config(config);
    let stored = auth
        .store_provider_token(
            &provider,
            profile_name,
            &token,
            metadata,
            set_active.unwrap_or(true),
        )
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        summarize_auth_profile(&stored),
        "provider credentials stored",
    ))
}

pub async fn remove_provider_credentials(
    config: &Config,
    provider: &str,
    profile: Option<&str>,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let profile_name = profile_name_or_default(profile);
    let auth = AuthService::from_config(config);
    let removed = auth
        .remove_profile(provider, profile_name)
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        json!({
            "removed": removed,
            "provider": provider,
            "profile": profile_name,
        }),
        "provider credentials removed",
    ))
}

pub async fn list_provider_credentials(
    config: &Config,
    provider_filter: Option<String>,
) -> Result<RpcOutcome<Vec<super::responses::AuthProfileSummary>>, String> {
    let auth = AuthService::from_config(config);
    let profiles = auth.load_profiles().map_err(|e| e.to_string())?;
    let mut items = profiles
        .profiles
        .values()
        .filter(|profile| profile.provider != APP_SESSION_PROVIDER)
        .filter(|profile| {
            provider_filter
                .as_ref()
                .is_none_or(|provider| profile.provider == *provider)
        })
        .map(summarize_auth_profile)
        .collect::<Vec<_>>();
    items.sort_by(|a, b| {
        a.provider
            .cmp(&b.provider)
            .then_with(|| a.profile_name.cmp(&b.profile_name))
    });

    Ok(RpcOutcome::single_log(items, "provider credentials listed"))
}

pub async fn oauth_connect(
    config: &Config,
    provider: &str,
    skill_id: Option<&str>,
    response_type: Option<&str>,
    encryption_mode: Option<&str>,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let api_url = effective_api_url(&config.api_url);
    let token = get_session_token(config)?.ok_or_else(|| {
        "session JWT required; complete login and store_session first".to_string()
    })?;
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let r = client
        .connect(provider, &token, skill_id, response_type, encryption_mode)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        serde_json::json!({ "oauthUrl": r.oauth_url, "state": r.state }),
        "oauth connect URL ready",
    ))
}

pub async fn oauth_list_integrations(
    config: &Config,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let api_url = effective_api_url(&config.api_url);
    let token = get_session_token(config)?.ok_or_else(|| "session JWT required".to_string())?;
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let list = client
        .list_integrations(&token)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        serde_json::to_value(&list).map_err(|e| e.to_string())?,
        "integrations listed",
    ))
}

pub async fn oauth_fetch_integration_tokens(
    config: &Config,
    integration_id: &str,
    encryption_key: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let api_url = effective_api_url(&config.api_url);
    let token = get_session_token(config)?.ok_or_else(|| "session JWT required".to_string())?;
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let tokens = client
        .fetch_integration_tokens_handoff(integration_id, &token, encryption_key)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        serde_json::to_value(&tokens).map_err(|e| e.to_string())?,
        "integration tokens retrieved",
    ))
}

pub async fn oauth_fetch_client_key(
    config: &Config,
    integration_id: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let api_url = effective_api_url(&config.api_url);
    let token = get_session_token(config)?.ok_or_else(|| "session JWT required".to_string())?;
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let client_key = client
        .fetch_client_key(integration_id, &token)
        .await
        .map_err(|e| e.to_string())?;
    log::debug!(
        "[credentials] client key retrieved for integration {}",
        integration_id
    );
    Ok(RpcOutcome::single_log(
        json!({ "clientKey": client_key, "integrationId": integration_id }),
        "client key retrieved (one-time handoff)",
    ))
}

pub async fn oauth_revoke_integration(
    config: &Config,
    integration_id: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let api_url = effective_api_url(&config.api_url);
    let token = get_session_token(config)?.ok_or_else(|| "session JWT required".to_string())?;
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    client
        .revoke_integration(integration_id, &token)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        serde_json::json!({ "revoked": true, "integrationId": integration_id }),
        "integration revoked",
    ))
}
