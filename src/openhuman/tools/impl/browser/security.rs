use std::net::ToSocketAddrs;
use std::time::Duration;

pub(crate) fn normalize_domains(domains: Vec<String>) -> Vec<String> {
    domains
        .into_iter()
        .map(|d| d.trim().to_lowercase())
        .filter(|d| !d.is_empty())
        .collect()
}

pub(crate) fn endpoint_reachable(endpoint: &reqwest::Url, timeout: Duration) -> bool {
    let host = match endpoint.host_str() {
        Some(host) if !host.is_empty() => host,
        _ => return false,
    };

    let port = match endpoint.port_or_known_default() {
        Some(port) => port,
        None => return false,
    };

    let mut addrs = match (host, port).to_socket_addrs() {
        Ok(addrs) => addrs,
        Err(_) => return false,
    };

    let addr = match addrs.next() {
        Some(addr) => addr,
        None => return false,
    };

    std::net::TcpStream::connect_timeout(&addr, timeout).is_ok()
}

pub(crate) fn extract_host(url_str: &str) -> anyhow::Result<String> {
    // Simple host extraction without url crate
    let url = url_str.trim();
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .or_else(|| url.strip_prefix("file://"))
        .unwrap_or(url);

    // Extract host — handle bracketed IPv6 addresses like [::1]:8080
    let authority = without_scheme.split('/').next().unwrap_or(without_scheme);

    let host = if authority.starts_with('[') {
        // IPv6: take everything up to and including the closing ']'
        authority.find(']').map_or(authority, |i| &authority[..=i])
    } else {
        // IPv4 or hostname: take everything before the port separator
        authority.split(':').next().unwrap_or(authority)
    };

    if host.is_empty() {
        anyhow::bail!("Invalid URL: no host");
    }

    Ok(host.to_lowercase())
}

pub(crate) fn is_private_host(host: &str) -> bool {
    // Strip brackets from IPv6 addresses like [::1]
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);

    if bare == "localhost" || bare.ends_with(".localhost") {
        return true;
    }

    // .local TLD (mDNS)
    if bare
        .rsplit('.')
        .next()
        .is_some_and(|label| label == "local")
    {
        return true;
    }

    // Parse as IP address to catch all representations (decimal, hex, octal, mapped)
    if let Ok(ip) = bare.parse::<std::net::IpAddr>() {
        return match ip {
            std::net::IpAddr::V4(v4) => is_non_global_v4(v4),
            std::net::IpAddr::V6(v6) => is_non_global_v6(v6),
        };
    }

    false
}

/// Returns `true` for any IPv4 address that is not globally routable.
pub(crate) fn is_non_global_v4(v4: std::net::Ipv4Addr) -> bool {
    let [a, b, _, _] = v4.octets();
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.is_broadcast()
        || v4.is_multicast()
        // Shared address space (100.64/10)
        || (a == 100 && (64..=127).contains(&b))
        // Reserved (240.0.0.0/4)
        || a >= 240
        // Documentation (192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24)
        || (a == 192 && b == 0)
        || (a == 198 && b == 51)
        || (a == 203 && b == 0)
        // Benchmarking (198.18.0.0/15)
        || (a == 198 && (18..=19).contains(&b))
}

/// Returns `true` for any IPv6 address that is not globally routable.
pub(crate) fn is_non_global_v6(v6: std::net::Ipv6Addr) -> bool {
    let segs = v6.segments();
    v6.is_loopback()
        || v6.is_unspecified()
        || v6.is_multicast()
        // Unique-local (fc00::/7) — IPv6 equivalent of RFC 1918
        || (segs[0] & 0xfe00) == 0xfc00
        // Link-local (fe80::/10)
        || (segs[0] & 0xffc0) == 0xfe80
        // IPv4-mapped addresses
        || v6.to_ipv4_mapped().is_some_and(is_non_global_v4)
}

pub(crate) fn allow_all_browser_domains() -> bool {
    matches!(
        std::env::var("OPENHUMAN_BROWSER_ALLOW_ALL").ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

pub(crate) fn host_matches_allowlist(host: &str, allowed: &[String]) -> bool {
    allowed.iter().any(|pattern| {
        if pattern == "*" {
            return true;
        }
        if pattern.starts_with("*.") {
            // Wildcard subdomain match
            let suffix = &pattern[1..]; // ".example.com"
            host.ends_with(suffix) || host == &pattern[2..]
        } else {
            // Exact match or subdomain
            host == pattern || host.ends_with(&format!(".{pattern}"))
        }
    })
}
