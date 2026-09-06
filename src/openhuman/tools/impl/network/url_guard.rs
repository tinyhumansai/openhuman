//! Shared URL validation + SSRF guards for outbound network tools.
//!
//! Used by `http_request`, `curl`, and any future tool that takes a
//! user-supplied URL. Two allowlist modes:
//!
//! - **Open allowlist** (`allowed_domains` is empty): any public non-private
//!   host is permitted. All SSRF guards still apply (loopback / RFC1918 /
//!   link-local / multicast / documentation / shared-address /
//!   IPv4-mapped IPv6, `localhost` / `*.localhost` / `*.local`).
//! - **Strict allowlist** (`allowed_domains` is non-empty): only the listed
//!   domains and their subdomains are permitted.
//!
//! Both modes enforce: http(s) only, no whitespace, no userinfo, no IPv6 hosts.
//!
//! **Alternate IP notations** (octal, hex, decimal): Rust's `IpAddr::parse`
//! rejects them so they are treated as plain hostnames. In strict-allowlist
//! mode they are rejected by the domain check. In open-allowlist mode they
//! pass `validate_url` but are caught by `validate_url_with_dns_check`
//! because they fail real-world DNS resolution.
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
pub fn validate_url(raw_url: &str, allowed_domains: &[String]) -> anyhow::Result<String> {
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

    let host = extract_host(url)?;

    if is_private_or_local_host(&host) {
        log::debug!(
            "[url_guard] ssrf block: host={host} mode={}",
            if allowed_domains.is_empty() {
                "open"
            } else {
                "strict"
            }
        );
        anyhow::bail!("Blocked local/private host: {host}");
    }

    // Empty allowed_domains = open mode: any public non-private host is
    // permitted (same as ["*"]). This ensures the http_request tool works
    // out of the box regardless of whether the user configured an explicit
    // domain list, and keeps web-fetch consistent across routing paths.
    // A non-empty list = strict mode: only listed domains pass. (#2700)
    if !allowed_domains.is_empty() && !host_matches_allowlist(&host, allowed_domains) {
        log::debug!(
            "[url_guard] strict-allowlist rejection: host={host} allowed={:?}",
            allowed_domains
        );
        anyhow::bail!(
            "I'm not allowed to open '{host}' — it isn't in your allowed websites. \
             Add it (or turn on \"Allow all sites\") under \
             Settings → Advanced → Search engine → Allowed websites, then ask me again."
        );
    }

    log::debug!(
        "[url_guard] validate_url ok: host={host} mode={}",
        if allowed_domains.is_empty() {
            "open"
        } else {
            "strict"
        }
    );

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
pub(crate) async fn validate_url_with_dns_check(
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

pub fn normalize_allowed_domains(domains: Vec<String>) -> Vec<String> {
    if domains.is_empty() {
        return Vec::new();
    }
    let mut normalized = domains
        .into_iter()
        .filter_map(|d| normalize_domain(&d))
        .collect::<Vec<_>>();
    normalized.sort_unstable();
    normalized.dedup();
    if normalized.is_empty() {
        // All entries were malformed (whitespace-only, scheme-only, etc.) and
        // filtered out. Returning empty would silently enter open mode; instead
        // return a sentinel that keeps the tool in strict mode and rejects every
        // URL — fail-closed on misconfiguration. (#2738)
        log::warn!(
            "[url_guard] all configured allowed_domains entries are invalid — \
             treating as misconfigured allowlist (fail-closed)"
        );
        return vec!["<misconfigured-allowlist>".to_string()];
    }
    normalized
}

pub fn normalize_domain(raw: &str) -> Option<String> {
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

pub fn extract_host(url: &str) -> anyhow::Result<String> {
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

pub fn extract_port(url: &str) -> anyhow::Result<u16> {
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

pub fn host_matches_allowlist(host: &str, allowed_domains: &[String]) -> bool {
    allowed_domains.iter().any(|domain| {
        // `"*"` is the explicit allow-all wildcard (the "Allow all sites"
        // toggle), mirroring the browser tool. Local/private hosts are still
        // rejected upstream by `is_private_or_local_host`, so a wildcard only
        // opens *public* hosts, never the loopback/RFC1918 SSRF surface.
        domain == "*"
            || host == domain
            || host
                .strip_suffix(domain)
                .is_some_and(|prefix| prefix.ends_with('.'))
    })
}

pub fn is_private_or_local_host(host: &str) -> bool {
    let unbracketed = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    let bare = unbracketed.strip_suffix('.').unwrap_or(unbracketed);

    let lower = bare.to_ascii_lowercase();

    let has_local_tld = lower
        .rsplit('.')
        .next()
        .is_some_and(|label| label == "local");

    if lower == "localhost" || lower.ends_with(".localhost") || has_local_tld {
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

pub fn is_non_global_v4(v4: std::net::Ipv4Addr) -> bool {
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
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || (a == 198 && (18..=19).contains(&b))
        // 0.0.0.0/8 — "this network" (RFC 1122 §3.2.1.3). `is_unspecified()` only
        // covers 0.0.0.0 itself, but the whole /8 routes to the local host on
        // Linux. Carried over from the `ops_install` copy this replaces.
        || a == 0
}

pub fn is_non_global_v6(v6: std::net::Ipv6Addr) -> bool {
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
#[path = "url_guard_tests.rs"]
mod tests;
