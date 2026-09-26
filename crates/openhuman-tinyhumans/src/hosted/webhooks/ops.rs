//! Webhook tunnel RPC ops — thin adapters over the SDK's `webhooks()` client.
//!
//! Every call authenticates with the core's resolved backend credential
//! ([`HostedClient`]); the backend owns tunnel ownership and quotas. Input
//! validation runs before the credential is resolved.

use serde_json::Value;
use tinyhumans_sdk::api::webhooks::{CreateWebhookTunnelRequest, UpdateWebhookTunnelRequest};

use openhuman_core::config::Config;
use openhuman_core::rpc::RpcOutcome;

use crate::hosted::client::HostedClient;

fn require_id(id: &str) -> Result<&str, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("id is required".to_string());
    }
    Ok(id)
}

/// `GET /webhooks/core` — the user's tunnels.
pub async fn list_tunnels(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "GET /webhooks/core",
        client.sdk().webhooks().list_tunnels().await,
    )?;
    Ok(RpcOutcome::single_log(data, "webhook tunnels fetched"))
}

/// `POST /webhooks/core`. `name` is trimmed and required; a blank
/// `description` is dropped rather than sent.
pub async fn create_tunnel(
    config: &Config,
    name: &str,
    description: Option<String>,
) -> Result<RpcOutcome<Value>, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("name is required".to_string());
    }
    let request = CreateWebhookTunnelRequest {
        name: name.to_string(),
        description: description
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty()),
    };
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "POST /webhooks/core",
        client.sdk().webhooks().create_tunnel(&request).await,
    )?;
    Ok(RpcOutcome::single_log(data, "webhook tunnel created"))
}

/// `GET /webhooks/core/{id}`.
pub async fn get_tunnel(config: &Config, id: &str) -> Result<RpcOutcome<Value>, String> {
    let id = require_id(id)?;
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "GET /webhooks/core/{id}",
        client.sdk().webhooks().get_tunnel(id).await,
    )?;
    Ok(RpcOutcome::single_log(data, "webhook tunnel fetched"))
}

/// `PATCH /webhooks/core/{id}`; omitted fields are left unchanged.
pub async fn update_tunnel(
    config: &Config,
    id: &str,
    request: UpdateWebhookTunnelRequest,
) -> Result<RpcOutcome<Value>, String> {
    let id = require_id(id)?;
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "PATCH /webhooks/core/{id}",
        client.sdk().webhooks().update_tunnel(id, &request).await,
    )?;
    Ok(RpcOutcome::single_log(data, "webhook tunnel updated"))
}

/// `DELETE /webhooks/core/{id}`.
pub async fn delete_tunnel(config: &Config, id: &str) -> Result<RpcOutcome<Value>, String> {
    let id = require_id(id)?;
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "DELETE /webhooks/core/{id}",
        client.sdk().webhooks().delete_tunnel(id).await,
    )?;
    Ok(RpcOutcome::single_log(data, "webhook tunnel deleted"))
}

/// `GET /webhooks/core/bandwidth` — the remaining bandwidth budget.
pub async fn get_bandwidth(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "GET /webhooks/core/bandwidth",
        client.sdk().webhooks().get_bandwidth().await,
    )?;
    Ok(RpcOutcome::single_log(data, "webhook bandwidth fetched"))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
