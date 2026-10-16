//! The Composio backend route.
//!
//! `composio.mode = "backend"` sends Composio calls through the TinyHumans
//! backend's proxy. The core decides nothing about that mode itself: it asks
//! the route builder and proxy-target resolver registered here
//! ([`register`]). With this crate absent the mode answers
//! `BACKEND_UNAVAILABLE:`.

use openhuman_embed::__host::composio::{register_composio_route_builder, ComposioRoute};
#[cfg(feature = "modules")]
use openhuman_embed::__host::composio::{register_proxy_target, ProxyTarget};
use openhuman_embed::__host::config::schema::COMPOSIO_MODE_BACKEND;
use openhuman_embed::__host::config::Config;

use super::build_client;

/// The message when the backend mode has no usable credential. Wording is
/// load-bearing: the core's session-expiry classifier recognises "no backend
/// session token" and demotes it to an expected user-state error.
const NO_SESSION: &str =
    "composio backend mode unavailable: no backend session token. Sign in or set a TinyHumans API key.";

/// The proxy target for `config`: the backend origin and the credential secret.
#[cfg(feature = "modules")]
fn proxy_target(config: &Config) -> Result<ProxyTarget, String> {
    let client = build_client(config).ok_or_else(|| NO_SESSION.to_string())?;
    Ok(ProxyTarget {
        base_url: client.backend_url.clone(),
        auth_token: client.auth_token.clone(),
    })
}

/// Registers the backend route builder and proxy target with the core.
/// Idempotent.
pub(crate) fn register() {
    register_composio_route_builder(COMPOSIO_MODE_BACKEND, |config| {
        if build_client(config).is_none() {
            anyhow::bail!("{NO_SESSION}");
        }
        log::debug!("[tinyhumans] composio: resolved backend route");
        Ok(ComposioRoute::Backend)
    });
    #[cfg(feature = "modules")]
    register_proxy_target(COMPOSIO_MODE_BACKEND, proxy_target);
}
