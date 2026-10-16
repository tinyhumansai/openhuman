use crate::core::runtime::DispatchCtx;
use crate::platform::socket::models::SocketState;

use super::SocketManager;

async fn connect_static_using(
    manager: &SocketManager,
    url: &str,
    token: &str,
) -> Result<SocketState, String> {
    let _rebind = manager.lock_identity_rebind().await;
    manager.disconnect().await?;
    manager.connect(url, token).await?;
    Ok(manager.get_state())
}

pub async fn connect_static(
    manager: &SocketManager,
    url: &str,
    token: &str,
) -> Result<SocketState, String> {
    log::info!("[socket:rpc] connect");
    connect_static_using(manager, url, token).await
}

pub async fn disconnect(manager: &SocketManager) -> Result<SocketState, String> {
    log::info!("[socket:rpc] disconnect");
    let _rebind = manager.lock_identity_rebind().await;
    manager.disconnect().await?;
    Ok(manager.get_state())
}

/// Connect with a session token, reusing a live socket for the same identity.
/// The identity lock serializes concurrent bootstrap and RPC connections.
async fn connect_with_session_using(
    manager: &SocketManager,
    url: &str,
    token: &str,
    provider: super::token_provider::TokenProvider,
) -> Result<SocketState, String> {
    let _rebind = manager.lock_identity_rebind().await;
    if manager.is_live_for(url, token) {
        log::info!(
            "[socket:rpc] connect_with_session — {url} already connected with this session; reusing the live socket"
        );
        return Ok(manager.get_state());
    }
    manager.disconnect().await?;
    manager.connect_with_provider(url, provider).await?;
    Ok(manager.get_state())
}

pub async fn connect_with_session(
    ctx: &DispatchCtx,
    manager: &SocketManager,
) -> Result<SocketState, String> {
    log::info!("[socket:rpc] connect_with_session — resolving credentials");
    let config = std::sync::Arc::new(ctx.load_config().await?);
    let api_url = config
        .resolved_endpoints
        .require_control_plane()?
        .to_string();
    let token = crate::security::credentials::session_support::get_session_token(&config)
        .map_err(|e| format!("failed to read session token: {e}"))?
        .ok_or("no session token stored — user must log in first")?;
    if crate::security::credentials::session_support::is_local_session_token(&token) {
        return Err("offline local session cannot connect to hosted socket".to_string());
    }

    let provider =
        super::token_provider::token_provider_from_config(std::sync::Arc::clone(&config));
    connect_with_session_using(manager, &api_url, &token, provider).await
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
