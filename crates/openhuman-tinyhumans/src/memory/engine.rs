//! Binding the hosted `tinyhumans` memory engine.
//!
//! | | |
//! | --- | --- |
//! | Endpoint | `[memory.engines.tinyhumans] endpoint`, else the control-plane origin |
//! | Credential | the host's backend credential, resolved per request |
//! | Headers | the attribution headers (`x-sdk-name`, ...) every backend caller sends |
//! | Off when | signed out or on the offline local session |
//!
//! The layout root arrives in the [`EngineRequest`]; on the TinyHumans wire
//! the tenant root is pinned by the backend, so none is sent. The core's
//! engine cache keys built engines by the fingerprint formed here.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::memory_engines::{
    build_cached, rooted, BearerSource, Binding, EngineCredential, EngineRequest, EngineSettings,
    TINYHUMANS_ENGINE,
};
use openhuman_embed::__host::security::credentials::session_support::resolve_backend_credential;

use crate::backend::headers::attribution_headers;
use crate::backend::url::effective_backend_api_url;

fn off(endpoint: Option<String>, reason: &str) -> Binding {
    Binding::Off {
        engine: Some(TINYHUMANS_ENGINE.to_string()),
        endpoint,
        reason: reason.to_string(),
    }
}

/// The endpoint memory talks to for `config`.
pub(crate) fn endpoint_for(config: &Config) -> String {
    config
        .memory
        .endpoint_for(TINYHUMANS_ENGINE)
        .unwrap_or_else(|| effective_backend_api_url(&config.api_url))
}

/// Binds the hosted engine for `request.config` below `request.root`.
pub(crate) fn bind(request: &EngineRequest<'_>) -> Binding {
    let config = request.config;
    let endpoint = endpoint_for(config);
    if let Err(error) = resolve_backend_credential(config) {
        tracing::debug!(error = %error, "[tinyhumans][memory] no usable TinyHumans credential");
        return off(Some(endpoint), "sign in to use TinyHumans memory");
    }
    // The backend expects the host's attribution (`x-sdk-name`, ...) on every
    // request, as every other backend caller sends it.
    let headers = engine_headers();
    let fingerprint = format!(
        "{TINYHUMANS_ENGINE}|{endpoint}|{}|{headers:?}",
        config.config_path.display()
    );
    let source: Arc<dyn BearerSource> = Arc::new(HostBearer {
        config: Arc::new(config.clone()),
    });
    let (settings, layout) = rooted(
        EngineSettings {
            endpoint: Some(endpoint),
            headers,
            ..EngineSettings::default()
        },
        request.root,
        true,
        config.memory.legacy_user_segment_read,
    );
    build_cached(
        TINYHUMANS_ENGINE,
        settings,
        fingerprint + &layout,
        EngineCredential::Dynamic(source),
    )
}

/// The attribution headers as engine settings carry them. A value that is not
/// text is left out (and logged by name only).
fn engine_headers() -> BTreeMap<String, String> {
    let headers = match attribution_headers() {
        Ok(headers) => headers,
        Err(error) => {
            tracing::debug!(error = %error, "[tinyhumans][memory] attribution headers unavailable");
            return BTreeMap::new();
        }
    };
    headers
        .iter()
        .filter_map(|(name, value)| match value.to_str() {
            Ok(value) => Some((name.as_str().to_string(), value.to_string())),
            Err(_) => {
                tracing::debug!(header = %name, "[tinyhumans][memory] skipping a non-text attribution header");
                None
            }
        })
        .collect()
}

/// The host's backend credential as a per-request bearer.
pub(crate) struct HostBearer {
    pub(crate) config: Arc<Config>,
}

#[async_trait]
impl BearerSource for HostBearer {
    async fn bearer(&self) -> openhuman_embed::memory::api::Result<String> {
        match resolve_backend_credential(&self.config) {
            Ok(credential) => Ok(credential.into_secret()),
            Err(error) => {
                tracing::debug!(error = %error, "[tinyhumans][memory] backend credential unavailable");
                Err(openhuman_embed::memory::api::Error::Unauthorized(
                    "the backend credential is unavailable".to_string(),
                ))
            }
        }
    }
}
