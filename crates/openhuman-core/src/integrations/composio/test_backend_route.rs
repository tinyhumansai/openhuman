//! Test stand-in for a host's registered backend route.
//!
//! The core ships no backend-proxied Composio route of its own: a host
//! registers a route builder and a proxy target for `composio.mode = backend`.
//! Tests that exercise the backend mode against a local mock register this
//! stand-in, which points the proxy at `config.api_url` with the resolved
//! credential, the way a host would. Registrations are thread-local in
//! `cfg(test)` builds, so call it on the test's own thread.

use crate::config::schema::COMPOSIO_MODE_BACKEND;
use crate::config::Config;
use crate::modules::connectors::{register_proxy_target, ProxyTarget};

use super::client::{register_composio_route_builder, ComposioRoute};

const NO_SESSION: &str =
    "composio backend mode unavailable: no backend session token. Sign in or set a TinyHumans API key.";

fn target(config: &Config) -> Result<ProxyTarget, String> {
    use crate::security::credentials::session_support::{
        is_local_session_token, resolve_backend_credential, BackendCredential,
    };
    let credential = resolve_backend_credential(config).map_err(|_| NO_SESSION.to_string())?;
    if let BackendCredential::Session(token) = &credential {
        if is_local_session_token(token.trim()) {
            return Err(NO_SESSION.to_string());
        }
    }
    if credential.secret().trim().is_empty() {
        return Err(NO_SESSION.to_string());
    }
    Ok(ProxyTarget {
        base_url: config
            .api_url
            .clone()
            .unwrap_or_else(|| "http://127.0.0.1:1".to_string()),
        auth_token: credential.secret().trim().to_string(),
    })
}

/// Registers the stand-in route builder and proxy target on this thread.
pub(crate) fn register() {
    register_composio_route_builder(COMPOSIO_MODE_BACKEND, |config| {
        target(config)
            .map(|_| ComposioRoute::Backend)
            .map_err(|message| anyhow::anyhow!(message))
    });
    register_proxy_target(COMPOSIO_MODE_BACKEND, target);
}
