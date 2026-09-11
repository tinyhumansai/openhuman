
/// Derive the install directory slug from the SKILL.md frontmatter.
///
/// Prefers `metadata.id` (the spec-aligned identifier) when present. Falls
/// back to a sanitized form of `name`:
///   * lowercase ASCII
///   * non-alphanumeric runs collapsed to a single `-`
///   * leading/trailing `-` trimmed
///
/// Rejects the empty string and paths that would escape the skills root
/// (`..`, `/`, `\`). Max length is [`MAX_NAME_LEN`].
pub(crate) fn derive_install_slug(fm: &WorkflowFrontmatter) -> Result<String, String> {
    let candidate = fm
        .metadata
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| fm.name.clone());

    let mut out = String::with_capacity(candidate.len());
    let mut last_dash = false;
    for ch in candidate.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }

    if out.is_empty() {
        return Err(
            "invalid SKILL.md: cannot derive slug from empty name/id — set a value in frontmatter"
                .to_string(),
        );
    }
    if out.len() > MAX_NAME_LEN {
        return Err(format!(
            "invalid SKILL.md: derived slug {out:?} exceeds {MAX_NAME_LEN} chars"
        ));
    }
    if out.contains("..") || out.contains('/') || out.contains('\\') {
        return Err(format!(
            "invalid SKILL.md: derived slug {out:?} contains forbidden path components"
        ));
    }

    Ok(out)
}

/// Validate a remote skill install URL. Returns `Ok(())` when the URL is
/// well-formed, uses `https`, and points at a public host.
///
/// Rejects:
/// * empty string or > [`MAX_INSTALL_URL_LEN`] bytes
/// * non-`https` schemes (including `http`, `ftp`, `file`, `git+ssh`)
/// * missing or empty host
/// * `localhost`, `*.localhost`, `*.local`
/// * IPv4 literals in loopback (127.0.0.0/8), private (10/8, 172.16/12,
///   192.168/16), link-local (169.254/16), shared-address (100.64/10),
///   multicast, broadcast, or unspecified (0.0.0.0) ranges
/// * IPv6 literals in loopback (::1), unspecified (::), unique-local
///   (fc00::/7), link-local (fe80::/10), or multicast (ff00::/8)
pub fn validate_install_url(raw: &str) -> Result<(), String> {
    validate_install_url_with_config(raw, read_allow_local_http_env())
}

/// Same as [`validate_install_url`] but takes an explicit `allow_local_http`
/// flag instead of reading `OPENHUMAN_SKILL_INSTALL_ALLOW_LOCAL_HTTP` from the
/// process environment. Callers below the public entry point should always use
/// this variant — the env-var read in `validate_install_url` is racy across
/// threads under the parallel test runner (#4567), and the escape hatch is
/// only meaningful at a single boundary.
pub(crate) fn validate_install_url_with_config(
    raw: &str,
    allow_local_http: bool,
) -> Result<(), String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("url must not be empty".to_string());
    }
    if trimmed.len() > MAX_INSTALL_URL_LEN {
        return Err(format!(
            "url exceeds max {MAX_INSTALL_URL_LEN} chars (got {})",
            trimmed.len()
        ));
    }
    let parsed = url::Url::parse(trimmed).map_err(|e| format!("invalid url {trimmed:?}: {e}"))?;
    if parsed.scheme() != "https" {
        if allow_local_http && is_loopback_http_url(trimmed) {
            return Ok(());
        }
        return Err(format!(
            "url scheme {:?} not allowed; https only",
            parsed.scheme()
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| format!("url {trimmed:?} has no host"))?;
    if host.is_empty() {
        return Err(format!("url {trimmed:?} has empty host"));
    }
    if is_blocked_install_host(host) {
        return Err(format!(
            "host {host:?} not allowed (loopback/private/link-local/multicast)"
        ));
    }
    Ok(())
}

/// Reads the local-HTTP install escape hatch env var. Kept as the single
/// boundary point where env is inspected so downstream validation never
/// touches process-global state — required to stop the parallel-test race
/// described in #4567.
fn read_allow_local_http_env() -> bool {
    std::env::var(ALLOW_LOCAL_HTTP_ENV).ok().as_deref() == Some("1")
}

fn is_loopback_http_url(raw: &str) -> bool {
    let Ok(parsed) = url::Url::parse(raw) else {
        return false;
    };
    if parsed.scheme() != "http" {
        return false;
    }
    let Some(host) = parsed.host_str() else {
        return false;
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

/// Resolve the host in the given URL and reject if any returned IP falls in
/// loopback / private / link-local / multicast / unspecified ranges.
///
/// Covers the DNS-to-private-IP SSRF vector: a public-looking hostname can
/// still resolve to 127.0.0.1 / 169.254.x / fc00::/7 etc., which
/// [`validate_install_url`] alone cannot detect because it only inspects
/// literal IP hosts.
///
/// Caveat: does **not** close the DNS-rebinding gap. `reqwest` performs its
/// own DNS lookup on the GET below, and a rebinding server can answer the
/// check with a public IP and answer reqwest with a private one. Full
/// mitigation requires resolving to a `SocketAddr` here and passing it to
/// reqwest via a custom resolver that only honours the pinned address.
pub async fn validate_resolved_host(raw_url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(raw_url)
        .map_err(|e| format!("invalid url {raw_url:?} during DNS guard: {e}"))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| format!("url {raw_url:?} has no host (DNS guard)"))?;
    // `tokio::net::lookup_host` wants "host:port". Default https → 443.
    let port = parsed.port_or_known_default().unwrap_or(443);
    // IPv6 literal hosts come back bracketed from `url::Url`; `lookup_host`
    // needs the bracketed form for IPv6 to parse correctly.
    let lookup_target = if parsed
        .host()
        .map(|h| matches!(h, url::Host::Ipv6(_)))
        .unwrap_or(false)
    {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };

    tracing::debug!(
        host = %host,
        port = port,
        "[skills] validate_resolved_host: resolving"
    );

    let mut addrs = tokio::net::lookup_host(&lookup_target)
        .await
        .map_err(|e| format!("dns lookup failed for {host:?}: {e}"))?
        .peekable();
    if addrs.peek().is_none() {
        return Err(format!("host {host:?} resolved to no IP addresses"));
    }
    for addr in addrs {
        let ip = addr.ip();
        match ip {
            std::net::IpAddr::V4(v4) => {
                if is_private_v4(&v4) {
                    tracing::warn!(
                        host = %host,
                        resolved = %v4,
                        "[skills] validate_resolved_host: rejected private IPv4"
                    );
                    return Err(format!(
                        "host {host:?} resolved to non-public IPv4 {v4} (loopback/private/link-local)"
                    ));
                }
            }
            std::net::IpAddr::V6(v6) => {
                if is_private_v6(&v6) {
                    tracing::warn!(
                        host = %host,
                        resolved = %v6,
                        "[skills] validate_resolved_host: rejected private IPv6"
                    );
                    return Err(format!(
                        "host {host:?} resolved to non-public IPv6 {v6} (loopback/ula/link-local)"
                    ));
                }
            }
        }
    }
    Ok(())
}

fn is_blocked_install_host(host: &str) -> bool {
    let lower = host.to_ascii_lowercase();
    // url::Url::host_str returns IPv6 literals wrapped in brackets (e.g. "[::1]").
    // Strip them before attempting Ipv6Addr parse.
    let stripped = lower
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(&lower);
    if stripped == "localhost" || stripped.ends_with(".localhost") || stripped.ends_with(".local") {
        return true;
    }
    if let Ok(v4) = stripped.parse::<Ipv4Addr>() {
        return is_private_v4(&v4);
    }
    if let Ok(v6) = stripped.parse::<Ipv6Addr>() {
        return is_private_v6(&v6);
    }
    false
}

fn is_private_v4(ip: &Ipv4Addr) -> bool {
    if ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_unspecified()
        || ip.is_multicast()
    {
        return true;
    }
    let [a, b, _, _] = ip.octets();
    // 100.64.0.0/10 shared address (CGN)
    if a == 100 && (64..=127).contains(&b) {
        return true;
    }
    // 0.0.0.0/8
    if a == 0 {
        return true;
    }
    false
}

fn is_private_v6(ip: &Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return true;
    }
    let first = ip.segments()[0];
    // fc00::/7 unique-local
    if (first & 0xfe00) == 0xfc00 {
        return true;
    }
    // fe80::/10 link-local
    if (first & 0xffc0) == 0xfe80 {
        return true;
    }
    false
}
