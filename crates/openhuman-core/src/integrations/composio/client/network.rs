//! The host's proxy and TLS policy for Composio, as the module is told it.
//!
//! The module's HTTP client cannot read the host's runtime proxy settings or
//! its TLS store, so the host resolves its policy for the `tool.composio`
//! service for one destination and hands it over: through `Configure` for the
//! configured route, and on every stateless direct read.
//!
//! The decision stays here. Whether a proxy applies at all
//! (`ProxyConfig::should_apply_to_service`: enabled, and the scope covers this
//! service), which URL serves a destination (`all_proxy` first, then the one
//! for the destination's scheme, which is the order the host's own `reqwest`
//! clients try them), and which hosts bypass it. The module only applies the
//! result.

use tinyconnectors_bus::{ComposioTlsRoots, ComposioTransportConfig};

use crate::config::runtime_proxy_config;

/// The proxy service key Composio traffic is scoped under.
const SERVICE_KEY: &str = "tool.composio";

fn usable(url: Option<&String>) -> Option<String> {
    let url = url?.trim();
    if url.is_empty() {
        return None;
    }
    // The host's `reqwest` clients ignore a proxy URL they cannot parse (and
    // warn); do the same instead of failing every Composio call over it.
    match reqwest::Proxy::all(url) {
        Ok(_) => Some(url.to_string()),
        Err(error) => {
            tracing::warn!(error = %error, "[composio-direct] ignoring an unusable proxy URL");
            None
        }
    }
}

/// The network policy for a request to `base_url`, or `None` when it is the
/// module's default (no proxy of the host's choosing, bundled roots).
pub(crate) fn module_transport(base_url: &str) -> Option<ComposioTransportConfig> {
    module_transport_with_proxy(base_url, &runtime_proxy_config())
}

/// [`module_transport`] against an explicit proxy policy instead of the
/// process-global runtime one, so a caller (or a test) that already holds the
/// policy does not race every config load that republishes the global.
pub(crate) fn module_transport_with_proxy(
    base_url: &str,
    proxy: &crate::config::schema::ProxyConfig,
) -> Option<ComposioTransportConfig> {
    let proxy_url = if proxy.should_apply_to_service(SERVICE_KEY) {
        let https = base_url.trim().to_ascii_lowercase().starts_with("https://");
        usable(proxy.all_proxy.as_ref()).or_else(|| {
            if https {
                usable(proxy.https_proxy.as_ref())
            } else {
                usable(proxy.http_proxy.as_ref())
            }
        })
    } else {
        None
    };
    let tls_roots = if cfg!(target_os = "windows") {
        ComposioTlsRoots::Platform
    } else {
        ComposioTlsRoots::Bundled
    };

    let config = ComposioTransportConfig {
        no_proxy: if proxy_url.is_some() {
            proxy.normalized_no_proxy()
        } else {
            Vec::new()
        },
        proxy_url,
        tls_roots,
    };
    tracing::debug!(
        proxied = config.proxy_url.is_some(),
        no_proxy_entries = config.no_proxy.len(),
        platform_roots = config.tls_roots == ComposioTlsRoots::Platform,
        "[composio-direct] module transport policy resolved"
    );
    (config != ComposioTransportConfig::default()).then_some(config)
}

#[cfg(test)]
#[path = "network_tests.rs"]
mod tests;
