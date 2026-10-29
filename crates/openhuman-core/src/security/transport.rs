//! Credential transport checks shared by host-owned network adapters.

/// Validate provider URL structure before saving or creating a network client.
///
/// WHATWG-like parsers may silently strip ASCII control characters; reject them
/// before parsing to preserve the exact configuration the operator supplied.
pub(crate) fn is_valid_provider_endpoint(endpoint: &str) -> bool {
    if endpoint.bytes().any(|byte| byte <= 0x1f || byte == 0x7f) {
        return false;
    }
    let value = endpoint.trim();
    if value.is_empty() || value.contains('?') || value.contains('#') {
        return false;
    }
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https")
        && url.host().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
}

/// Whether an HTTP endpoint can safely carry bearer or equivalent credentials.
/// HTTPS endpoints are allowed; HTTP is limited to actual loopback addresses.
pub(crate) fn is_safe_endpoint_for_bearer(endpoint: &str) -> bool {
    if !is_valid_provider_endpoint(endpoint) {
        return false;
    }
    let Ok(url) = url::Url::parse(endpoint.trim()) else {
        return false;
    };
    match url.scheme() {
        "https" => true,
        "http" => match url.host() {
            Some(url::Host::Domain("localhost")) => true,
            Some(url::Host::Ipv4(address)) => address.is_loopback(),
            Some(url::Host::Ipv6(address)) => address.is_loopback(),
            _ => false,
        },
        _ => false,
    }
}

/// Classify known HTTP request headers that carry authentication state.
/// Do not log values or use permissive prefix matching on arbitrary headers.
pub(crate) fn has_credential_bearing_headers(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(name, value)| {
        !value.trim().is_empty()
            && matches!(
                name.trim().to_ascii_lowercase().as_str(),
                "authorization"
                    | "proxy-authorization"
                    | "x-api-key"
                    | "api-key"
                    | "x-goog-api-key"
                    | "x-anthropic-api-key"
                    | "anthropic-api-key"
                    | "x-auth-token"
                    | "x-access-token"
                    | "x-aws-security-token"
                    | "cookie"
            )
    })
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
