//! Shared URL validation + SSRF guards for outbound network tools.
//!
//! Used by `http_request`, `curl`, and any future tool that takes a
//! user-supplied URL. The contract is intentionally strict:
//!
//! - http(s) only
//! - non-empty allowlist required (callers pass it in)
//! - no whitespace, no userinfo, no IPv6 hosts
//! - blocks loopback / RFC1918 / link-local / multicast / documentation /
//!   shared-address / IPv4-mapped IPv6, including `localhost` /
//!   `*.localhost` / `*.local`
//!
//! The blocklist deliberately does NOT cover alternate IP notations
//! (octal, hex, decimal) because Rust's `IpAddr::parse` rejects them —
//! they fall through and get rejected by the allowlist instead. See the
//! tests in `http_request.rs` for the documented behaviour.
//!
//! ## DNS Rebinding Protection
//!
//! Hostname validation alone is insufficient: an attacker can register a
//! domain that alternates DNS responses between a public IP (passing the
//! allowlist) and a private IP (e.g. 127.0.0.1). To close this gap,
//! callers should use [`validate_url_with_dns_check`] which resolves the
//! hostname and re-validates the resolved IPs before the request is made.

use std::future::Future;
use std::net::{IpAddr, ToSocketAddrs};

/// Validate a URL against the allowlist + SSRF rules. Returns the
/// original URL on success.
pub(super) fn validate_url(raw_url: &str, allowed_domains: &[String]) -> anyhow::Result<String> {
    let url = raw_url.trim();

    if url.is_empty() {
        anyhow::bail!("URL cannot be empty");
    }

    if url.chars().any(char::is_whitespace) {
        anyhow::bail!("URL cannot contain whitespace");
    }

    if !url.starts_with("http://") && !url.starts_with("https://") {
        anyhow::bail!("Only http:// and https:// URLs are allowed");
    }

    if allowed_domains.is_empty() {
        anyhow::bail!(
            "Network tool is enabled but no allowed_domains are configured. Add [http_request].allowed_domains in config.toml"
        );
    }

    let host = extract_host(url)?;

    if is_private_or_local_host(&host) {
        anyhow::bail!("Blocked local/private host: {host}");
    }

    if !host_matches_allowlist(&host, allowed_domains) {
        anyhow::bail!("Host '{host}' is not in http_request.allowed_domains");
    }

    Ok(url.to_string())
}

/// Like [`validate_url`] but also resolves the hostname via DNS and
/// verifies that none of the resolved IPs are private/local. This
/// defends against DNS rebinding attacks where an attacker's domain
/// initially resolves to a public IP (passing the allowlist) and then
/// flips to 127.0.0.1 at request time.
///
/// Callers should use this function instead of `validate_url` in all
/// paths that make outbound HTTP requests.
pub(super) async fn validate_url_with_dns_check(
    raw_url: &str,
    allowed_domains: &[String],
) -> anyhow::Result<String> {
    validate_url_with_dns_check_with_resolver(raw_url, allowed_domains, resolve_host_ips).await
}

async fn validate_url_with_dns_check_with_resolver<F, Fut>(
    raw_url: &str,
    allowed_domains: &[String],
    resolver: F,
) -> anyhow::Result<String>
where
    F: FnOnce(String, u16) -> Fut,
    Fut: Future<Output = anyhow::Result<Vec<IpAddr>>>,
{
    let url = validate_url(raw_url, allowed_domains)?;

    let host = extract_host(&url)?;

    // If the host is already a valid IP literal, `is_private_or_local_host`
    // has already checked it above. We only need DNS resolution for hostnames.
    if host.parse::<std::net::IpAddr>().is_ok() {
        return Ok(url);
    }

    let port = extract_port(&url)?;
    log::debug!("[url_guard] resolving DNS for host={host} port={port}");
    let addrs = resolver(host.clone(), port).await?;

    if addrs.is_empty() {
        anyhow::bail!("DNS resolution returned no addresses for '{host}'");
    }

    log::debug!("[url_guard] DNS resolved host={host} addrs={}", addrs.len());

    for addr in &addrs {
        let ip_str = addr.to_string();
        if is_private_or_local_host(&ip_str) {
            log::debug!("[url_guard] DNS rebinding blocked host={host} resolved_ip={ip_str}");
            anyhow::bail!(
                "DNS rebinding blocked: '{host}' resolved to private/local address {ip_str}"
            );
        }
    }

    Ok(url)
}

async fn resolve_host_ips(host: String, port: u16) -> anyhow::Result<Vec<IpAddr>> {
    let log_host = host.clone();
    tokio::task::spawn_blocking(move || {
        (host.as_str(), port)
            .to_socket_addrs()
            .map_err(|e| {
                log::debug!("[url_guard] DNS resolution failed host={host} port={port} error={e}");
                anyhow::anyhow!("DNS resolution failed for '{host}': {e}")
            })
            .map(|iter| iter.map(|addr| addr.ip()).collect())
    })
    .await
    .map_err(|e| {
        log::debug!("[url_guard] DNS resolution task failed host={log_host} port={port} error={e}");
        anyhow::anyhow!("DNS resolution task failed for '{log_host}': {e}")
    })?
}

pub(super) fn normalize_allowed_domains(domains: Vec<String>) -> Vec<String> {
    let mut normalized = domains
        .into_iter()
        .filter_map(|d| normalize_domain(&d))
        .collect::<Vec<_>>();
    normalized.sort_unstable();
    normalized.dedup();
    normalized
}

pub(super) fn normalize_domain(raw: &str) -> Option<String> {
    let mut d = raw.trim().to_lowercase();
    if d.is_empty() {
        return None;
    }

    if let Some(stripped) = d.strip_prefix("https://") {
        d = stripped.to_string();
    } else if let Some(stripped) = d.strip_prefix("http://") {
        d = stripped.to_string();
    }

    if let Some((host, _)) = d.split_once('/') {
        d = host.to_string();
    }

    d = d.trim_start_matches('.').trim_end_matches('.').to_string();

    if let Some((host, _)) = d.split_once(':') {
        d = host.to_string();
    }

    if d.is_empty() || d.chars().any(char::is_whitespace) {
        return None;
    }

    Some(d)
}

pub(super) fn extract_host(url: &str) -> anyhow::Result<String> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .ok_or_else(|| anyhow::anyhow!("Only http:// and https:// URLs are allowed"))?;

    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .ok_or_else(|| anyhow::anyhow!("Invalid URL"))?;

    if authority.is_empty() {
        anyhow::bail!("URL must include a host");
    }

    if authority.contains('@') {
        anyhow::bail!("URL userinfo is not allowed");
    }

    if authority.starts_with('[') {
        anyhow::bail!("IPv6 hosts are not supported in http_request");
    }

    let host = authority
        .split(':')
        .next()
        .unwrap_or_default()
        .trim()
        .trim_end_matches('.')
        .to_lowercase();

    if host.is_empty() {
        anyhow::bail!("URL must include a valid host");
    }

    Ok(host)
}

fn extract_port(url: &str) -> anyhow::Result<u16> {
    let is_http = url.starts_with("http://");
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .ok_or_else(|| anyhow::anyhow!("Only http:// and https:// URLs are allowed"))?;

    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .ok_or_else(|| anyhow::anyhow!("Invalid URL"))?;

    if authority.starts_with('[') {
        anyhow::bail!("IPv6 hosts are not supported in http_request");
    }

    if let Some((_, port)) = authority.rsplit_once(':') {
        if port.is_empty() || !port.chars().all(|ch| ch.is_ascii_digit()) {
            anyhow::bail!("URL port must be numeric");
        }
        return port
            .parse::<u16>()
            .map_err(|_| anyhow::anyhow!("URL port is out of range"));
    }

    Ok(if is_http { 80 } else { 443 })
}

pub(super) fn host_matches_allowlist(host: &str, allowed_domains: &[String]) -> bool {
    allowed_domains.iter().any(|domain| {
        host == domain
            || host
                .strip_suffix(domain)
                .is_some_and(|prefix| prefix.ends_with('.'))
    })
}

pub(super) fn is_private_or_local_host(host: &str) -> bool {
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);

    let has_local_tld = bare
        .rsplit('.')
        .next()
        .is_some_and(|label| label == "local");

    if bare == "localhost" || bare.ends_with(".localhost") || has_local_tld {
        return true;
    }

    if let Ok(ip) = bare.parse::<std::net::IpAddr>() {
        return match ip {
            std::net::IpAddr::V4(v4) => is_non_global_v4(v4),
            std::net::IpAddr::V6(v6) => is_non_global_v6(v6),
        };
    }

    false
}

fn is_non_global_v4(v4: std::net::Ipv4Addr) -> bool {
    let [a, b, c, _] = v4.octets();
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.is_broadcast()
        || v4.is_multicast()
        || (a == 100 && (64..=127).contains(&b))
        || a >= 240
        || (a == 192 && b == 0 && (c == 0 || c == 2))
        || (a == 198 && b == 51)
        || (a == 203 && b == 0)
        || (a == 198 && (18..=19).contains(&b))
}

fn is_non_global_v6(v6: std::net::Ipv6Addr) -> bool {
    let segs = v6.segments();
    v6.is_loopback()
        || v6.is_unspecified()
        || v6.is_multicast()
        || (segs[0] & 0xfe00) == 0xfc00
        || (segs[0] & 0xffc0) == 0xfe80
        || (segs[0] == 0x2001 && segs[1] == 0x0db8)
        || v6.to_ipv4_mapped().is_some_and(is_non_global_v4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_domain_strips_scheme_path_and_case() {
        let got = normalize_domain("  HTTPS://Docs.Example.com/path ").unwrap();
        assert_eq!(got, "docs.example.com");
    }

    #[test]
    fn normalize_allowed_domains_deduplicates() {
        let got = normalize_allowed_domains(vec![
            "example.com".into(),
            "EXAMPLE.COM".into(),
            "https://example.com/".into(),
        ]);
        assert_eq!(got, vec!["example.com".to_string()]);
    }

    #[test]
    fn validate_accepts_exact_domain() {
        let allow = vec!["example.com".to_string()];
        let got = validate_url("https://example.com/docs", &allow).unwrap();
        assert_eq!(got, "https://example.com/docs");
    }

    #[test]
    fn validate_accepts_http() {
        let allow = vec!["example.com".to_string()];
        assert!(validate_url("http://example.com", &allow).is_ok());
    }

    #[test]
    fn validate_accepts_subdomain() {
        let allow = vec!["example.com".to_string()];
        assert!(validate_url("https://api.example.com/v1", &allow).is_ok());
    }

    #[test]
    fn validate_rejects_allowlist_miss() {
        let allow = vec!["example.com".to_string()];
        let err = validate_url("https://google.com", &allow)
            .unwrap_err()
            .to_string();
        assert!(err.contains("allowed_domains"));
    }

    #[test]
    fn validate_rejects_localhost() {
        let allow = vec!["localhost".to_string()];
        let err = validate_url("https://localhost:8080", &allow)
            .unwrap_err()
            .to_string();
        assert!(err.contains("local/private"));
    }

    #[test]
    fn validate_rejects_private_ipv4() {
        let allow = vec!["192.168.1.5".to_string()];
        let err = validate_url("https://192.168.1.5", &allow)
            .unwrap_err()
            .to_string();
        assert!(err.contains("local/private"));
    }

    #[test]
    fn validate_rejects_whitespace() {
        let allow = vec!["example.com".to_string()];
        let err = validate_url("https://example.com/hello world", &allow)
            .unwrap_err()
            .to_string();
        assert!(err.contains("whitespace"));
    }

    #[test]
    fn validate_rejects_userinfo() {
        let allow = vec!["example.com".to_string()];
        let err = validate_url("https://user@example.com", &allow)
            .unwrap_err()
            .to_string();
        assert!(err.contains("userinfo"));
    }

    #[test]
    fn validate_requires_allowlist() {
        let err = validate_url("https://example.com", &[])
            .unwrap_err()
            .to_string();
        assert!(err.contains("allowed_domains"));
    }

    #[test]
    fn validate_rejects_ftp_scheme() {
        let allow = vec!["example.com".to_string()];
        let err = validate_url("ftp://example.com", &allow)
            .unwrap_err()
            .to_string();
        assert!(err.contains("http://") || err.contains("https://"));
    }

    #[test]
    fn validate_rejects_empty_url() {
        let allow = vec!["example.com".to_string()];
        let err = validate_url("", &allow).unwrap_err().to_string();
        assert!(err.contains("empty"));
    }

    #[test]
    fn validate_rejects_ipv6_host() {
        let allow = vec!["example.com".to_string()];
        let err = validate_url("http://[::1]:8080/path", &allow)
            .unwrap_err()
            .to_string();
        assert!(err.contains("IPv6"));
    }

    #[test]
    fn blocks_multicast_ipv4() {
        assert!(is_private_or_local_host("224.0.0.1"));
        assert!(is_private_or_local_host("239.255.255.255"));
    }

    #[test]
    fn blocks_broadcast() {
        assert!(is_private_or_local_host("255.255.255.255"));
    }

    #[test]
    fn blocks_reserved_ipv4() {
        assert!(is_private_or_local_host("240.0.0.1"));
        assert!(is_private_or_local_host("250.1.2.3"));
    }

    #[test]
    fn blocks_documentation_ranges() {
        assert!(is_private_or_local_host("192.0.2.1"));
        assert!(is_private_or_local_host("198.51.100.1"));
        assert!(is_private_or_local_host("203.0.113.1"));
    }

    #[test]
    fn blocks_benchmarking_range() {
        assert!(is_private_or_local_host("198.18.0.1"));
        assert!(is_private_or_local_host("198.19.255.255"));
    }

    #[test]
    fn blocks_ipv6_localhost() {
        assert!(is_private_or_local_host("::1"));
        assert!(is_private_or_local_host("[::1]"));
    }

    #[test]
    fn blocks_ipv6_multicast() {
        assert!(is_private_or_local_host("ff02::1"));
    }

    #[test]
    fn blocks_ipv6_link_local() {
        assert!(is_private_or_local_host("fe80::1"));
    }

    #[test]
    fn blocks_ipv6_unique_local() {
        assert!(is_private_or_local_host("fd00::1"));
    }

    #[test]
    fn blocks_ipv4_mapped_ipv6() {
        assert!(is_private_or_local_host("::ffff:127.0.0.1"));
        assert!(is_private_or_local_host("::ffff:192.168.1.1"));
        assert!(is_private_or_local_host("::ffff:10.0.0.1"));
    }

    #[test]
    fn allows_public_ipv4() {
        assert!(!is_private_or_local_host("8.8.8.8"));
        assert!(!is_private_or_local_host("1.1.1.1"));
        assert!(!is_private_or_local_host("93.184.216.34"));
    }

    #[test]
    fn blocks_ipv6_documentation_range() {
        assert!(is_private_or_local_host("2001:db8::1"));
    }

    #[test]
    fn allows_public_ipv6() {
        assert!(!is_private_or_local_host("2607:f8b0:4004:800::200e"));
    }

    #[test]
    fn blocks_shared_address_space() {
        assert!(is_private_or_local_host("100.64.0.1"));
        assert!(is_private_or_local_host("100.127.255.255"));
        assert!(!is_private_or_local_host("100.63.0.1"));
        assert!(!is_private_or_local_host("100.128.0.1"));
    }

    #[test]
    fn ssrf_blocks_loopback_127_range() {
        assert!(is_private_or_local_host("127.0.0.1"));
        assert!(is_private_or_local_host("127.0.0.2"));
        assert!(is_private_or_local_host("127.255.255.255"));
    }

    #[test]
    fn ssrf_blocks_rfc1918_10_range() {
        assert!(is_private_or_local_host("10.0.0.1"));
        assert!(is_private_or_local_host("10.255.255.255"));
    }

    #[test]
    fn ssrf_blocks_rfc1918_172_range() {
        assert!(is_private_or_local_host("172.16.0.1"));
        assert!(is_private_or_local_host("172.31.255.255"));
    }

    #[test]
    fn ssrf_blocks_unspecified_address() {
        assert!(is_private_or_local_host("0.0.0.0"));
    }

    #[test]
    fn ssrf_blocks_dot_localhost_subdomain() {
        assert!(is_private_or_local_host("evil.localhost"));
        assert!(is_private_or_local_host("a.b.localhost"));
    }

    #[test]
    fn ssrf_blocks_dot_local_tld() {
        assert!(is_private_or_local_host("service.local"));
    }

    #[test]
    fn ssrf_ipv6_unspecified() {
        assert!(is_private_or_local_host("::"));
    }

    // ── Defense-in-depth: alternate IP notations rejected by allowlist
    //
    // Rust's IpAddr::parse() rejects octal, hex, decimal, and
    // zero-padded notations. They fall through as hostnames and get
    // rejected by the allowlist instead. These tests pin that
    // behaviour so a parser change can't silently re-open SSRF.

    #[test]
    fn ssrf_octal_loopback_not_parsed_as_ip() {
        assert!(!is_private_or_local_host("0177.0.0.1"));
    }

    #[test]
    fn ssrf_hex_loopback_not_parsed_as_ip() {
        assert!(!is_private_or_local_host("0x7f000001"));
    }

    #[test]
    fn ssrf_decimal_loopback_not_parsed_as_ip() {
        assert!(!is_private_or_local_host("2130706433"));
    }

    #[test]
    fn ssrf_zero_padded_loopback_not_parsed_as_ip() {
        assert!(!is_private_or_local_host("127.000.000.001"));
    }

    #[test]
    fn ssrf_alternate_notations_rejected_by_validate_url() {
        let allow = vec!["example.com".to_string()];
        for notation in [
            "http://0177.0.0.1",
            "http://0x7f000001",
            "http://2130706433",
            "http://127.000.000.001",
        ] {
            let err = validate_url(notation, &allow).unwrap_err().to_string();
            assert!(
                err.contains("allowed_domains"),
                "Expected allowlist rejection for {notation}, got: {err}"
            );
        }
    }

    // ── DNS rebinding protection ─────────────────────────────────

    #[tokio::test]
    async fn dns_check_blocks_localhost_resolution() {
        // "localhost" resolves to 127.0.0.1 on most systems. Even if
        // someone adds it to the allowlist, the DNS check should block it.
        let allow = vec!["localhost".to_string()];
        // validate_url itself already blocks "localhost" via the hostname check,
        // but validate_url_with_dns_check should also catch it.
        let err = validate_url_with_dns_check("https://localhost", &allow)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("local/private") || err.contains("rebinding"),
            "Expected SSRF block for localhost, got: {err}"
        );
    }

    #[tokio::test]
    async fn dns_check_passes_for_public_resolved_ip() {
        let allow = vec!["example.com".to_string()];
        let got = validate_url_with_dns_check_with_resolver(
            "https://example.com",
            &allow,
            |host, port| async move {
                assert_eq!(host, "example.com");
                assert_eq!(port, 443);
                Ok(vec!["93.184.216.34".parse().unwrap()])
            },
        )
        .await
        .unwrap();
        assert_eq!(got, "https://example.com");
    }

    #[tokio::test]
    async fn dns_check_blocks_private_resolved_ip() {
        let allow = vec!["example.com".to_string()];
        let err = validate_url_with_dns_check_with_resolver(
            "https://example.com",
            &allow,
            |_, _| async { Ok(vec!["127.0.0.1".parse().unwrap()]) },
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("DNS rebinding blocked"));
    }

    #[tokio::test]
    async fn dns_check_uses_explicit_port_for_resolution() {
        let allow = vec!["api.example.com".to_string()];
        let got = validate_url_with_dns_check_with_resolver(
            "http://api.example.com:8080/status",
            &allow,
            |host, port| async move {
                assert_eq!(host, "api.example.com");
                assert_eq!(port, 8080);
                Ok(vec!["93.184.216.34".parse().unwrap()])
            },
        )
        .await
        .unwrap();
        assert_eq!(got, "http://api.example.com:8080/status");
    }

    #[tokio::test]
    async fn dns_check_returns_resolver_failure() {
        let allow = vec!["example.com".to_string()];
        let err = validate_url_with_dns_check_with_resolver(
            "https://example.com",
            &allow,
            |host, _| async move {
                anyhow::bail!("DNS resolution failed for '{host}': resolver unavailable")
            },
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("DNS resolution failed"));
    }

    #[tokio::test]
    async fn dns_check_rejects_ip_literal_private() {
        let allow = vec!["10.0.0.1".to_string()];
        let err = validate_url_with_dns_check("https://10.0.0.1", &allow)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("local/private"));
    }
}
