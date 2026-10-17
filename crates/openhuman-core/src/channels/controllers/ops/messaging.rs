//! Channel messaging, reactions, and thread management.

use serde_json::Value;

use crate::backend::BackendClient;
use crate::config::Config;
use crate::core::Outcome;
use crate::security::credentials::session_support::resolve_backend_credential;

/// Send a reaction to a message in a channel via the backend API.
pub async fn channel_send_reaction(
    config: &Config,
    channel: &str,
    reaction: Value,
) -> Result<Outcome<Value>, String> {
    let api_url = crate::backend::require_base_url(&config.api_url)?;
    let jwt = resolve_backend_credential(config)?;

    log::debug!(
        "[channels] sending reaction to channel '{}' via {}",
        channel,
        api_url
    );

    let client = BackendClient::new(&api_url).map_err(|e| e.to_string())?;
    let result = client
        .send_channel_reaction(channel, &jwt, reaction)
        .await
        .map_err(|e| {
            format!(
                "failed to send channel reaction: {}",
                crate::backend::flatten_authed_error(e)
            )
        })?;

    log::debug!("[channels] send_reaction response: {:?}", result);

    Ok(Outcome::new(result, vec![]))
}

/// Create a thread in a channel via the backend API.
pub async fn channel_create_thread(
    config: &Config,
    channel: &str,
    title: &str,
) -> Result<Outcome<Value>, String> {
    let api_url = crate::backend::require_base_url(&config.api_url)?;
    let jwt = resolve_backend_credential(config)?;

    log::debug!(
        "[channels] creating thread in channel '{}' title='{}' via {}",
        channel,
        title,
        api_url
    );

    let client = BackendClient::new(&api_url).map_err(|e| e.to_string())?;
    let result = client
        .create_channel_thread(channel, &jwt, title)
        .await
        .map_err(|e| {
            format!(
                "failed to create channel thread: {}",
                crate::backend::flatten_authed_error(e)
            )
        })?;

    log::debug!("[channels] create_thread response: {:?}", result);

    Ok(Outcome::new(result, vec![]))
}

/// Close or reopen a thread in a channel via the backend API.
pub async fn channel_update_thread(
    config: &Config,
    channel: &str,
    thread_id: &str,
    action: &str,
) -> Result<Outcome<Value>, String> {
    let api_url = crate::backend::require_base_url(&config.api_url)?;
    let jwt = resolve_backend_credential(config)?;

    log::debug!(
        "[channels] updating thread '{}' in channel '{}' action='{}' via {}",
        thread_id,
        channel,
        action,
        api_url
    );

    let client = BackendClient::new(&api_url).map_err(|e| e.to_string())?;
    let result = client
        .update_channel_thread(channel, &jwt, thread_id, action)
        .await
        .map_err(|e| {
            format!(
                "failed to update channel thread: {}",
                crate::backend::flatten_authed_error(e)
            )
        })?;

    log::debug!("[channels] update_thread response: {:?}", result);

    Ok(Outcome::new(result, vec![]))
}

/// List threads in a channel via the backend API.
pub async fn channel_list_threads(
    config: &Config,
    channel: &str,
    active: Option<bool>,
) -> Result<Outcome<Value>, String> {
    let api_url = crate::backend::require_base_url(&config.api_url)?;
    let jwt = resolve_backend_credential(config)?;

    log::debug!(
        "[channels] listing threads in channel '{}' active={:?} via {}",
        channel,
        active,
        api_url
    );

    let client = BackendClient::new(&api_url).map_err(|e| e.to_string())?;
    let result = client
        .list_channel_threads(channel, &jwt, active)
        .await
        .map_err(|e| {
            format!(
                "failed to list channel threads: {}",
                crate::backend::flatten_authed_error(e)
            )
        })?;

    log::debug!("[channels] list_threads response: {:?}", result);

    Ok(Outcome::new(result, vec![]))
}
