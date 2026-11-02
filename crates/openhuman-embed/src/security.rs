//! Security checks shared by embedding entry points.

/// Whether an endpoint may receive a bearer credential.
///
/// HTTPS is accepted. Plain HTTP is limited to `localhost` and IP loopback
/// addresses; other schemes and malformed URLs are rejected.
pub(crate) fn is_safe_endpoint_for_bearer(endpoint: &str) -> bool {
    let Ok(url) = url::Url::parse(endpoint) else {
        return false;
    };

    match url.scheme() {
        "https" => true,
        "http" => match url.host() {
            Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
            Some(url::Host::Ipv4(address)) => address.is_loopback(),
            Some(url::Host::Ipv6(address)) => address.is_loopback(),
            None => false,
        },
        _ => false,
    }
}

#[cfg(test)]
#[path = "security_tests.rs"]
mod tests;
