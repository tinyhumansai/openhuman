//! Backend-brokered OAuth connect flow and integration token retrieval.
//!
//! Validation runs first, then the credential is resolved ([`HostedClient`]):
//! a user with no TinyHumans account gets the core's sentinel without a
//! request. Tokens, keys and encrypted blobs are never logged.

use serde_json::{json, Value};
use tinyhumans_sdk::api::types::IntegrationTokenRequest;

use openhuman_core::api::decrypt_handoff_blob;
use openhuman_core::config::Config;
use openhuman_core::rpc::RpcOutcome;

use super::types::{IntegrationSummary, IntegrationTokensHandoff};
use crate::hosted::client::HostedClient;

const LOG_PREFIX: &str = "[hosted][oauth]";

fn require_provider(provider: &str) -> Result<&str, String> {
    let p = provider.trim().trim_matches('/');
    if p.is_empty() {
        return Err("provider is required".to_string());
    }
    Ok(p)
}

fn require_integration_id(integration_id: &str) -> Result<&str, String> {
    let id = integration_id.trim();
    if id.is_empty() || id.len() != 24 {
        return Err("integrationId must be a 24-char hex id".to_string());
    }
    Ok(id)
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value.filter(|v| !v.is_empty()).map(str::to_string)
}

/// `GET /auth/{provider}/connect` — an authorize URL plus CSRF `state`.
pub async fn oauth_connect(
    config: &Config,
    provider: &str,
    skill_id: Option<&str>,
    response_type: Option<&str>,
    encryption_mode: Option<&str>,
) -> Result<RpcOutcome<Value>, String> {
    let provider = require_provider(provider)?;
    let client = HostedClient::from_config(config)?;
    let query = [
        ("skillId", non_empty(skill_id)),
        ("responseType", non_empty(response_type)),
        ("encryptionMode", non_empty(encryption_mode)),
    ];
    log::debug!("{LOG_PREFIX} connect provider={provider}");
    let value = client.finish_value(
        "GET /auth/{provider}/connect",
        client.sdk().auth().oauth_connect(provider, &query).await,
    )?;
    let oauth_url = value
        .get("oauthUrl")
        .or_else(|| value.get("oauth_url"))
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .ok_or_else(|| "auth connect request: missing oauthUrl in response".to_string())?;
    let state = value
        .get("state")
        .and_then(Value::as_str)
        .filter(|state| !state.is_empty())
        .ok_or_else(|| "auth connect request: missing state".to_string())?;
    Ok(RpcOutcome::single_log(
        json!({ "oauthUrl": oauth_url, "state": state }),
        "oauth connect URL ready",
    ))
}

/// `GET /auth/integrations` — the user's active integrations.
pub async fn oauth_list_integrations(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let client = HostedClient::from_config(config)?;
    let value = client.finish_value(
        "GET /auth/integrations",
        client.sdk().auth().list_integrations().await,
    )?;
    let integrations = value
        .get("integrations")
        .cloned()
        .unwrap_or_else(|| value.clone());
    let list: Vec<IntegrationSummary> = serde_json::from_value(integrations)
        .map_err(|e| format!("parse integrations response: {e}"))?;
    Ok(RpcOutcome::single_log(
        serde_json::to_value(&list).map_err(|e| e.to_string())?,
        "integrations listed",
    ))
}

/// `POST /auth/integrations/{id}/tokens` — a one-time handoff of the
/// integration's OAuth tokens, encrypted by the backend with `encryption_key`
/// and decrypted here. The key goes back to the backend as given (never
/// re-encoded) so base64url keys stay base64url on the wire.
pub async fn oauth_fetch_integration_tokens(
    config: &Config,
    integration_id: &str,
    encryption_key: &str,
) -> Result<RpcOutcome<Value>, String> {
    let id = require_integration_id(integration_id)?;
    let client = HostedClient::from_config(config)?;
    let request = IntegrationTokenRequest {
        key: encryption_key.trim().to_string(),
    };
    let value = client.finish_value(
        "POST /auth/integrations/{integrationId}/tokens",
        client
            .sdk()
            .auth()
            .create_integration_token(id, &request)
            .await,
    )?;
    let encrypted = value
        .get("encrypted")
        .and_then(Value::as_str)
        .ok_or_else(|| "integration tokens response missing encrypted payload".to_string())?;
    let plaintext = decrypt_handoff_blob(encrypted, encryption_key.trim())
        .map_err(|e| format!("integration tokens handoff: {e:#}"))?;
    let tokens: IntegrationTokensHandoff =
        serde_json::from_str(&plaintext).map_err(|e| format!("parse decrypted token JSON: {e}"))?;
    Ok(RpcOutcome::single_log(
        serde_json::to_value(&tokens).map_err(|e| e.to_string())?,
        "integration tokens retrieved",
    ))
}

/// `DELETE /auth/integrations/{id}`.
pub async fn oauth_revoke_integration(
    config: &Config,
    integration_id: &str,
) -> Result<RpcOutcome<Value>, String> {
    let id = integration_id.trim();
    if id.is_empty() {
        return Err("integration id is required".to_string());
    }
    let client = HostedClient::from_config(config)?;
    client.finish_value(
        "DELETE /auth/integrations/{integrationId}",
        client.sdk().auth().delete_integration(id).await,
    )?;
    Ok(RpcOutcome::single_log(
        json!({ "revoked": true, "integrationId": integration_id }),
        "integration revoked",
    ))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
