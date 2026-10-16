//! Where a hosted credential may be sent.
//!
//! A TinyHumans API key belongs to TinyHumans: only the managed hosts (or a
//! loopback endpoint for local development) are trusted recipients. Every
//! credential, session or key, additionally requires HTTPS, except to
//! loopback. These are the guards `BackendClient` and [`IntegrationsHttp`]
//! (and, in later stages, the voice and inference callers) run before a
//! credential is attached to a request.
//!
//! [`IntegrationsHttp`]: super::IntegrationsHttp

/// The managed hosts an API key may be sent to over HTTPS.
pub const MANAGED_API_KEY_HOSTS: [&str; 2] = ["api.tinyhumans.ai", "staging-api.tinyhumans.ai"];

fn is_loopback_host(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain("localhost")) => true,
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        _ => false,
    }
}

/// Whether `endpoint` is safe to carry a backend credential as a bearer.
///
/// `https://` always qualifies; plain `http://` only for loopback. A
/// plaintext non-loopback endpoint would put the credential on the wire in the
/// clear (CWE-319). Deliberately narrow to the managed-credential path:
/// `normalize_api_base_url` stays permissive because a library host's BYOK or
/// local `api_url` can legitimately be plain HTTP.
pub fn is_safe_endpoint_for_managed_bearer(endpoint: &str) -> bool {
    let Ok(url) = url::Url::parse(endpoint) else {
        return false;
    };
    match url.scheme() {
        "https" => true,
        "http" => is_loopback_host(&url),
        _ => false,
    }
}

/// Whether `endpoint` may receive a TinyHumans API key: the managed hosts over
/// HTTPS, or a loopback endpoint for local development. An arbitrary TLS host
/// is not a trusted key recipient.
pub fn is_managed_endpoint_for_api_key(endpoint: &str) -> bool {
    let Ok(url) = url::Url::parse(endpoint) else {
        return false;
    };
    if !is_safe_endpoint_for_managed_bearer(endpoint) {
        return false;
    }
    let managed = url.scheme() == "https"
        && url
            .host_str()
            .is_some_and(|h| MANAGED_API_KEY_HOSTS.contains(&h));
    managed || is_loopback_host(&url)
}

/// Reject `endpoint` unless it may carry `is_api_key`'s credential kind.
/// The error text is the one the core's callers have always surfaced.
pub fn validate_credential_endpoint(endpoint: &str, is_api_key: bool) -> anyhow::Result<()> {
    if is_api_key && !is_managed_endpoint_for_api_key(endpoint) {
        anyhow::bail!("TinyHumans API key requires the managed backend or a loopback endpoint");
    }
    if !is_safe_endpoint_for_managed_bearer(endpoint) {
        anyhow::bail!("backend credential requires HTTPS or a loopback HTTP endpoint");
    }
    Ok(())
}

#[cfg(test)]
#[path = "endpoint_tests.rs"]
mod tests;
