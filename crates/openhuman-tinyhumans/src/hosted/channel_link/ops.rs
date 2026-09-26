//! Channel-link token issuance and the managed Telegram / Discord link flows.
//!
//! Validation first, then the credential ([`HostedClient`]), so a user with no
//! TinyHumans account gets the core's sentinel without a request. Link tokens
//! are logged by length only.

use serde_json::Value;

use openhuman_core::config::Config;
use openhuman_core::rpc::RpcOutcome;

use crate::hosted::client::HostedClient;

/// Validate and normalise a link-token channel id (`telegram` | `discord`).
fn normalize_channel(channel: &str) -> Result<String, String> {
    let channel = channel.trim();
    if channel.is_empty() {
        return Err("channel is required".to_string());
    }
    let channel = channel.to_lowercase();
    if !matches!(channel.as_str(), "telegram" | "discord") {
        return Err(format!("unsupported channel: {channel}"));
    }
    Ok(channel)
}

/// `POST /auth/channels/{channel}/link-token` with an existing client.
pub(super) async fn link_token_payload(
    client: &HostedClient,
    channel: &str,
) -> Result<Value, String> {
    client.finish_value(
        "POST /auth/channels/{channel}/link-token",
        client.sdk().auth().create_channel_link_token(channel).await,
    )
}

/// Create a short-lived link token for the managed Telegram or Discord bot.
pub async fn auth_create_channel_link_token(
    config: &Config,
    channel: &str,
) -> Result<RpcOutcome<Value>, String> {
    let channel = normalize_channel(channel)?;
    let client = HostedClient::from_config(config)?;
    let payload = link_token_payload(&client, &channel).await?;
    Ok(RpcOutcome::single_log(
        payload,
        "channel link token created",
    ))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
