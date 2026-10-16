//! The G5 config enricher: fills `Config.resolved_endpoints` from this
//! crate's URL, header and product modules, so the core reads the resolved
//! control-plane origin, socket and tracing URLs, attribution headers and
//! product identity from config instead of owning them.
//!
//! | `ResolvedEndpoints` field | Source |
//! | --- | --- |
//! | `control_plane_url` | [`effective_backend_api_url`]`(&config.api_url)` |
//! | `inference_url` | [`effective_api_url`]`(&config.api_url)` |
//! | `socket_url` | the control-plane origin as a ready-to-dial Socket.IO URL |
//! | `tracing_ingest_url` | `<control>/telemetry/langfuse/ingestion` |
//! | `tracing_otlp_url` | `<control>/telemetry/langfuse/otel/v1/traces` |
//! | `deployment_environment` | [`deployment_environment`]`(<control>)` |
//! | `managed_hosts` | [`MANAGED_HOSTS`] |
//! | `attribution_headers` | [`attribution_headers`](crate::backend::headers::attribution_headers) |
//! | `product_identity` | [`product_identity`](crate::backend::product::product_identity) |
//!
//! `config.api_url` is honoured exactly as the core's `backend::base_url` /
//! `inference_base_url` honour it today: the transport's `base_url` is these
//! same two functions.

use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::config_enrichers::{register_enricher, ResolvedEndpoints};
use openhuman_embed::__host::util::url::join_url;

use crate::backend::headers::attribution_headers;
use crate::backend::product::{product_identity, product_identity_headers};
use crate::backend::url::{effective_api_url, effective_backend_api_url};

/// The id this enricher registers under.
pub const ENRICHER_ID: &str = "tinyhumans";

/// The Langfuse ingestion proxy route, relative to the control-plane origin.
/// The backend authenticates the session JWT, injects the Langfuse project
/// keys and forwards; clients POST here, never to `/api/public/ingestion`.
pub const TRACING_INGEST_PATH: &str = "/telemetry/langfuse/ingestion";

/// The Langfuse OTLP/HTTP traces route, relative to the control-plane origin.
pub const TRACING_OTLP_PATH: &str = "/telemetry/langfuse/otel/v1/traces";

/// The domain the deployed backends live under. A host outside it cannot be
/// one of ours, so it cannot be staging however it is spelled.
const DEPLOYMENT_DOMAIN: &str = "tinyhumans.ai";

/// The canonical production API host.
const PRODUCTION_HOST: &str = "api.tinyhumans.ai";

/// Host patterns this layer treats as its managed backend (exact host or
/// `.suffix`), including the legacy product hostnames and the bare
/// `openhuman` name used by compose setups. The core matches provider and
/// migration endpoints against these through
/// `ResolvedEndpoints::is_managed_host`.
pub const MANAGED_HOSTS: [&str; 5] = [
    "api.openhuman.ai",
    ".openhuman.ai",
    "api.tinyhumans.ai",
    ".tinyhumans.ai",
    "openhuman",
];

/// Classify a control-plane origin into the deployment bucket the core's
/// tracing push gate understands: `production` (the canonical API host),
/// `staging` (a `staging`-labelled host under the deployment domain) or
/// `external` (anything else, including unparseable input and non-HTTPS).
///
/// Loopback and private hosts are classified `development` by the core before
/// it consults this value. The host is parsed, never substring-matched:
/// `https://staging-attacker.invalid` is external, so the fail-closed gate
/// never pushes a session token to a host this tree does not own.
pub fn deployment_environment(base: &str) -> &'static str {
    let Ok(parsed) = url::Url::parse(base) else {
        return "external";
    };
    if parsed.scheme() != "https" {
        return "external";
    }
    let Some(url::Host::Domain(host)) = parsed.host() else {
        return "external";
    };
    let host = host.to_ascii_lowercase();
    let under_deployment_domain =
        host == DEPLOYMENT_DOMAIN || host.ends_with(&format!(".{DEPLOYMENT_DOMAIN}"));
    // Leftmost label only, so `staging-api...` and `staging...` match while
    // `api-staging-mirror...` does not sneak in on a substring.
    let leftmost_is_staging = host
        .split('.')
        .next()
        .is_some_and(|label| label == "staging" || label.starts_with("staging-"));
    if host == PRODUCTION_HOST {
        "production"
    } else if under_deployment_domain && leftmost_is_staging {
        "staging"
    } else {
        "external"
    }
}

/// The Socket.IO handshake URL for an HTTP(S) control-plane origin
/// (`https://host` -> `wss://host/socket.io/?EIO=4&transport=websocket`).
/// Returned unchanged when `base` does not parse.
pub fn socket_url_for(base: &str) -> String {
    let Ok(mut url) = url::Url::parse(base) else {
        return base.to_string();
    };
    let scheme = match url.scheme() {
        "https" => "wss",
        "http" => "ws",
        other => other,
    }
    .to_string();
    let _ = url.set_scheme(&scheme);
    url.set_path(&format!("{}/socket.io/", url.path().trim_end_matches('/')));
    url.set_query(Some("EIO=4&transport=websocket"));
    url.to_string()
}

fn header_pairs(headers: &reqwest::header::HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_string(), v.to_string()))
        })
        .collect()
}

/// Resolve every field for `config`. Pure apart from the product identity and
/// environment overrides the URL module reads.
pub fn resolve(config: &Config) -> ResolvedEndpoints {
    let control = effective_backend_api_url(&config.api_url);
    // A malformed attribution header degrades to the identity header alone,
    // as the transport does.
    let headers = attribution_headers().unwrap_or_else(|_| product_identity_headers());
    ResolvedEndpoints {
        socket_url: Some(socket_url_for(&control)),
        tracing_ingest_url: Some(join_url(&control, TRACING_INGEST_PATH)),
        tracing_otlp_url: Some(join_url(&control, TRACING_OTLP_PATH)),
        deployment_environment: Some(deployment_environment(&control).to_string()),
        managed_hosts: MANAGED_HOSTS.iter().map(|h| (*h).to_string()).collect(),
        inference_url: Some(effective_api_url(&config.api_url)),
        attribution_headers: header_pairs(&headers),
        product_identity: Some(product_identity().as_str().to_owned()),
        control_plane_url: Some(control),
    }
}

/// The enricher body: write the resolved endpoints onto `config`.
pub fn enrich(config: &mut Config) {
    let resolved = resolve(config);
    log::trace!(
        "[tinyhumans] config enricher: control_plane_url={:?}",
        resolved.control_plane_url
    );
    config.resolved_endpoints = resolved;
}

/// Registers the enricher with the core. Idempotent (re-registering replaces).
pub(crate) fn register() {
    register_enricher(ENRICHER_ID, enrich);
}

#[cfg(test)]
#[path = "enricher_tests.rs"]
mod tests;
