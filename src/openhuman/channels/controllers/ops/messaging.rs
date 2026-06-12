//! Channel messaging, reactions, and thread management.

use serde_json::Value;

use crate::api::config::effective_backend_api_url;
use crate::api::jwt::get_session_token;
use crate::api::rest::BackendOAuthClient;
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

/// Send a rich message to a channel via the backend API.
pub async fn channel_send_message(
    config: &Config,
    channel: &str,
    message: Value,
) -> Result<RpcOutcome<Value>, String> {
    let api_url = effective_backend_api_url(&config.api_url);
    let jwt = get_session_token(config)?
        .ok_or_else(|| "session JWT required; complete login first".to_string())?;

    log::debug!(
        "[channels] sending message to channel '{}' via {}",
        channel,
        api_url
    );

    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let result = client
        .send_channel_message(channel, &jwt, message)
        .await
        .map_err(|e| format!("failed to send channel message: {e}"))?;

    log::debug!("[channels] send_message response: {:?}", result);

    Ok(RpcOutcome::new(result, vec![]))
}

/// Send a reaction to a message in a channel via the backend API.
pub async fn channel_send_reaction(
    config: &Config,
    channel: &str,
    reaction: Value,
) -> Result<RpcOutcome<Value>, String> {
    let api_url = effective_backend_api_url(&config.api_url);
    let jwt = get_session_token(config)?
        .ok_or_else(|| "session JWT required; complete login first".to_string())?;

    log::debug!(
        "[channels] sending reaction to channel '{}' via {}",
        channel,
        api_url
    );

    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let result = client
        .send_channel_reaction(channel, &jwt, reaction)
        .await
        .map_err(|e| format!("failed to send channel reaction: {e}"))?;

    log::debug!("[channels] send_reaction response: {:?}", result);

    Ok(RpcOutcome::new(result, vec![]))
}

/// Create a thread in a channel via the backend API.
pub async fn channel_create_thread(
    config: &Config,
    channel: &str,
    title: &str,
) -> Result<RpcOutcome<Value>, String> {
    let api_url = effective_backend_api_url(&config.api_url);
    let jwt = get_session_token(config)?
        .ok_or_else(|| "session JWT required; complete login first".to_string())?;

    log::debug!(
        "[channels] creating thread in channel '{}' title='{}' via {}",
        channel,
        title,
        api_url
    );

    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let result = client
        .create_channel_thread(channel, &jwt, title)
        .await
        .map_err(|e| format!("failed to create channel thread: {e}"))?;

    log::debug!("[channels] create_thread response: {:?}", result);

    Ok(RpcOutcome::new(result, vec![]))
}

/// Close or reopen a thread in a channel via the backend API.
pub async fn channel_update_thread(
    config: &Config,
    channel: &str,
    thread_id: &str,
    action: &str,
) -> Result<RpcOutcome<Value>, String> {
    let api_url = effective_backend_api_url(&config.api_url);
    let jwt = get_session_token(config)?
        .ok_or_else(|| "session JWT required; complete login first".to_string())?;

    log::debug!(
        "[channels] updating thread '{}' in channel '{}' action='{}' via {}",
        thread_id,
        channel,
        action,
        api_url
    );

    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let result = client
        .update_channel_thread(channel, &jwt, thread_id, action)
        .await
        .map_err(|e| format!("failed to update channel thread: {e}"))?;

    log::debug!("[channels] update_thread response: {:?}", result);

    Ok(RpcOutcome::new(result, vec![]))
}

/// List threads in a channel via the backend API.
pub async fn channel_list_threads(
    config: &Config,
    channel: &str,
    active: Option<bool>,
) -> Result<RpcOutcome<Value>, String> {
    let api_url = effective_backend_api_url(&config.api_url);
    let jwt = get_session_token(config)?
        .ok_or_else(|| "session JWT required; complete login first".to_string())?;

    log::debug!(
        "[channels] listing threads in channel '{}' active={:?} via {}",
        channel,
        active,
        api_url
    );

    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let result = client
        .list_channel_threads(channel, &jwt, active)
        .await
        .map_err(|e| format!("failed to list channel threads: {e}"))?;

    log::debug!("[channels] list_threads response: {:?}", result);

    Ok(RpcOutcome::new(result, vec![]))
}
