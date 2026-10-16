//! Runtime-resolved service endpoints that a host fills in after a config load.
//!
//! The config file holds what the *user* chose. Some values the core needs are
//! not the user's to choose: the control-plane origin a host talks to, the
//! socket and tracing-ingestion URLs it derives, the attribution headers it
//! stamps on its requests, the product it reports as. A host that has such
//! values registers a config enricher
//! ([`register_enricher`](crate::config::register_enricher)); the loader runs it
//! on every load and it writes them here.
//!
//! The core reads these fields instead of owning the values. Like
//! [`EphemeralRoute`](super::ephemeral_route::EphemeralRoute) the field is
//! `#[serde(skip)]`: it never reaches `config.toml`, and nothing a caller
//! saves can persist it. With no enricher registered every field is empty and
//! the core behaves as it does with no hosted connection.

/// Endpoints and identity resolved by a registered config enricher. Every
/// field is `None`/empty until an enricher fills it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedEndpoints {
    /// The origin of the host's control plane (REST API base URL).
    pub control_plane_url: Option<String>,
    /// The origin inference requests go to, when it differs from the control
    /// plane.
    pub inference_url: Option<String>,
    /// The realtime socket URL.
    pub socket_url: Option<String>,
    /// Where tracing/observability events are pushed.
    pub tracing_ingest_url: Option<String>,
    /// Where OTLP trace exports are pushed (the OTLP/HTTP traces endpoint of
    /// the same tracing backend), when the host exposes one.
    pub tracing_otlp_url: Option<String>,
    /// The deployment bucket the host places its control plane in
    /// (`"production"`, `"staging"`, `"development"` or `"external"`). The core
    /// classifies loopback hosts itself and treats anything unrecognised as
    /// `"external"`, which does not push traces.
    pub deployment_environment: Option<String>,
    /// Host patterns the installed host treats as its own managed backend:
    /// an exact host (`api.example.com`) or a `.suffix` that matches any
    /// subdomain (`.example.com`). Empty with no host registered, so nothing
    /// is recognised as a managed endpoint.
    pub managed_hosts: Vec<String>,
    /// Headers stamped on every request to the control plane, `(name, value)`.
    pub attribution_headers: Vec<(String, String)>,
    /// The product identity the host reports (the `x-sdk-name` value).
    pub product_identity: Option<String>,
}

impl ResolvedEndpoints {
    /// Whether an enricher has filled nothing in.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The control-plane origin, or the `BACKEND_UNAVAILABLE:` sentinel
    /// `core::observability` demotes when no enricher resolved one, for
    /// callers on the JSON-RPC `String` error channel.
    pub fn require_control_plane(&self) -> Result<&str, String> {
        self.control_plane_url.as_deref().ok_or_else(|| {
            format!(
                "{} no control plane resolved",
                crate::core::observability::BACKEND_UNAVAILABLE_PREFIX
            )
        })
    }

    /// The inference origin: the resolved one, else the user's explicit
    /// `api_url` override (normalized), else `None`.
    pub fn inference_or_configured(&self, api_url: &Option<String>) -> Option<String> {
        self.inference_url.clone().or_else(|| {
            api_url
                .as_deref()
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .map(crate::util::url::normalize_api_base_url)
        })
    }

    /// Whether `url` (or a bare host) names one of the resolved managed hosts.
    /// Matching is on the host component only, case-insensitively, so a path
    /// or query that happens to contain a managed host name never matches.
    pub fn is_managed_host(&self, url: &str) -> bool {
        host_matches_any(url, &self.managed_hosts)
    }

    /// The attribution headers as a `reqwest` header map; invalid pairs are
    /// skipped.
    pub fn attribution_header_map(&self) -> reqwest::header::HeaderMap {
        let mut map = reqwest::header::HeaderMap::new();
        for (name, value) in &self.attribution_headers {
            if let (Ok(n), Ok(v)) = (
                reqwest::header::HeaderName::from_bytes(name.as_bytes()),
                reqwest::header::HeaderValue::from_str(value),
            ) {
                map.insert(n, v);
            }
        }
        map
    }
}

/// Lower-cased host of `url` (scheme, userinfo, port and path stripped).
fn host_of(url: &str) -> String {
    let lower = url.trim().to_ascii_lowercase();
    let without_scheme = lower.split("://").nth(1).unwrap_or(&lower);
    let authority = without_scheme.split('/').next().unwrap_or("");
    let host = authority.split('@').next_back().unwrap_or(authority);
    host.split(':').next().unwrap_or(host).to_string()
}

/// Whether the host of `url` equals a pattern or falls under a `.suffix`
/// pattern. Empty patterns never match.
pub fn host_matches_any(url: &str, patterns: &[String]) -> bool {
    if patterns.is_empty() {
        return false;
    }
    let host = host_of(url);
    if host.is_empty() {
        return false;
    }
    patterns.iter().any(|pattern| {
        let pattern = pattern.trim().to_ascii_lowercase();
        if pattern.is_empty() {
            false
        } else if pattern.starts_with('.') {
            host.ends_with(&pattern)
        } else {
            host == pattern
        }
    })
}

#[cfg(test)]
#[path = "resolved_endpoints_tests.rs"]
mod tests;
