//! Jev as OpenHuman manages it.
//!
//! The module uses the OpenHuman-managed Jev: a signed-in TinyHumans session
//! reaches Jev through the backend's System One proxy, billed to the account.
//! There is no other OpenHuman-side route.
//!
//! `None` means managed Jev is unavailable (signed out, or the offline local
//! session, which has no backend account); the `Evaluate` callback then
//! reports `Unsupported` and the module returns the original reply.

use std::sync::Arc;

use tinyinference_decisions::{Client, ClientConfig};

use crate::config::Config;
use crate::security::credentials::session_support::{
    is_local_session_token, resolve_backend_credential, BackendCredential,
};

pub(crate) fn managed(config: &Config) -> Option<Arc<Client>> {
    // Only a session token can be the offline local session; an API key is
    // never classified by its shape.
    let secret = match resolve_backend_credential(config).ok()? {
        BackendCredential::Session(token) if is_local_session_token(&token) => return None,
        BackendCredential::Session(token) | BackendCredential::ApiKey(token) => token,
    };
    let base = config.resolved_endpoints.control_plane_url.clone()?;
    let mut client = ClientConfig::tinyhumans_openrouter(secret);
    client.base_url = base;
    if let Some(identity) = config.resolved_endpoints.product_identity.clone() {
        client = client.with_sdk_name(&identity);
    }
    match Client::new(client) {
        Ok(c) => Some(Arc::new(c)),
        Err(e) => {
            log::warn!("[tinymemes] managed jev client build failed: {e}");
            None
        }
    }
}

#[cfg(test)]
#[path = "jev_tests.rs"]
mod tests;
