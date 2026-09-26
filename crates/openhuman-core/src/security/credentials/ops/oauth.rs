//! The one backend-brokered OAuth call the core still serves.
//!
//! `auth.oauth_connect`, `auth.oauth_list_integrations`,
//! `auth.oauth_fetch_integration_tokens` and `auth.oauth_revoke_integration`
//! moved to `openhuman-tinyhumans` (`hosted::oauth`, on the TinyHumans SDK).
//! `auth.oauth_fetch_client_key` stays here because its route
//! (`POST /auth/integrations/{id}/client-key`) has no SDK method and is absent
//! from the SDK's public-route registry; it moves once the route is added to
//! the SDK upstream.

use serde_json::json;

use crate::api::config::effective_backend_api_url;
use crate::api::jwt::get_session_token;
use crate::api::rest::BackendOAuthClient;
use crate::config::Config;
use crate::rpc::RpcOutcome;

pub async fn oauth_fetch_client_key(
    config: &Config,
    integration_id: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let api_url = effective_backend_api_url(&config.api_url);
    let token = get_session_token(config)?.ok_or_else(|| "session JWT required".to_string())?;
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    // `flatten_authed_error` keeps a 401 on the `SESSION_EXPIRED` sentinel (and
    // a missing transport on `BACKEND_UNAVAILABLE:`) instead of an opaque string.
    let client_key = client
        .fetch_client_key(integration_id, &token)
        .await
        .map_err(crate::api::flatten_authed_error)?;
    log::debug!(
        "[credentials] client key retrieved for integration {}",
        integration_id
    );
    Ok(RpcOutcome::single_log(
        json!({ "clientKey": client_key, "integrationId": integration_id }),
        "client key retrieved (one-time handoff)",
    ))
}
