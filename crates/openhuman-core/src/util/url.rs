//! URL helpers shared across domains: base-URL normalisation, safe path
//! joining and local-host classification. Nothing here knows about any
//! particular backend — hosted-backend URL resolution lives with the host that
//! registers the config enricher (`openhuman-tinyhumans`).

/// Whether the URL host is an on-device provider endpoint according to
/// TinySecurity's shared U5 classification. With security disabled, the host's
/// own loopback classification ([`loopback_host`]).
pub fn host_is_local(parsed: &::url::Url) -> bool {
    let Some(host) = parsed.host_str() else {
        return false;
    };
    provider_host_is_local(host)
}

pub fn provider_host_is_local(host: &str) -> bool {
    if crate::security::switch::disabled().is_some() {
        return loopback_host(host);
    }
    #[cfg(feature = "security-module")]
    {
        crate::modules::security::text::classify_provider_host_blocking(
            host.to_owned(),
            std::time::Duration::from_secs(2),
        )
        .unwrap_or(false)
    }
    #[cfg(not(feature = "security-module"))]
    {
        let _ = host;
        false
    }
}

/// Whether `host` names this machine: `localhost` or a `*.localhost` name, or a
/// loopback or unspecified IP literal (`127.0.0.0/8`, `0.0.0.0`, `::1`, `::`,
/// IPv4-mapped forms; bracketed or not). No DNS lookup. The same on-device rule
/// TinySecurity applies, used when security is disabled and the module is not
/// asked. A LAN or private address is not on-device, so local-only privacy
/// mode never treats it as local.
#[must_use]
pub fn loopback_host(host: &str) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    let on_device_v4 = |ip: std::net::Ipv4Addr| ip.is_loopback() || ip.is_unspecified();
    match host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<std::net::IpAddr>()
    {
        Ok(std::net::IpAddr::V4(ip)) => on_device_v4(ip),
        Ok(std::net::IpAddr::V6(ip)) => ip
            .to_ipv4_mapped()
            .map_or_else(|| ip.is_loopback() || ip.is_unspecified(), on_device_v4),
        Err(_) => false,
    }
}

/// Trim whitespace and strip trailing slashes so all base URLs are in
/// canonical form before being joined with a path.
///
/// This is deliberately a cheap string operation (no URL parsing) so it can
/// be called on potentially-invalid strings without panicking.
pub fn normalize_api_base_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

/// Like [`normalize_api_base_url`] but also **strips any inference-style path**
/// (e.g. `/v1/chat/completions`) so the result is always a bare host
/// root suitable as a backend base.
///
/// # Why this exists
///
/// Users (and CI configs) sometimes set `BACKEND_URL` or `config.api_url` to
/// the full inference endpoint. Backend callers append domain-specific paths
/// (`/auth/me`, `/integrations/…`) which then land on
/// `.../v1/chat/completions/auth/me` — an obvious 404.
///
/// # Scheme-less fallback
///
/// `option_env!`-baked values occasionally omit the scheme
/// (e.g. `api.example.com/v1/chat/completions`). We retry with an
/// `https://` prefix so the path can still be stripped before the value is
/// used as a base. Without this, a scheme-less inference path survived into
/// every backend call — Sentry `OPENHUMAN-TAURI-H6 / -HN`, issue #2075.
pub fn normalize_backend_api_base_url(url: &str) -> String {
    let normalized = normalize_api_base_url(url);
    if normalized.is_empty() {
        return normalized;
    }

    let parsed = ::url::Url::parse(&normalized)
        .or_else(|_| ::url::Url::parse(&format!("https://{normalized}")));

    let Ok(mut parsed) = parsed else {
        // Unparseable even with the scheme prefix — return as-is; the caller
        // will surface a network error rather than silently 404.
        return normalized;
    };

    // Strip everything after the host (path, query, fragment).
    if parsed.path() != "/" {
        parsed.set_path("");
    }
    parsed.set_query(None);
    parsed.set_fragment(None);

    parsed.to_string().trim_end_matches('/').to_string()
}

/// Safely join an API base URL with an absolute path.
///
/// # Behaviour
///
/// | `base`                                    | `path`                    | result                                                                 |
/// |-------------------------------------------|---------------------------|------------------------------------------------------------------------|
/// | `https://api.example.com`                 | `/auth/me`                | `https://api.example.com/auth/me`                                     |
/// | `https://api.example.com/v1/chat/…`       | `/integrations/foo`       | `https://api.example.com/integrations/foo`  ← path replaced           |
/// | `https://api.example.com`                 | `""`                      | `https://api.example.com`                                             |
/// | `not a url`                               | `/x`                      | `not a url/x`  ← safe fallback concat                                 |
///
/// Paths **must start with `/`**. Relative paths (no leading slash) are
/// resolved per RFC 3986 — the base's last segment is dropped — which is
/// almost never what an API client wants.
pub fn join_url(base: &str, path: &str) -> String {
    let base = base.trim();

    if path.is_empty() {
        return normalize_api_base_url(base);
    }

    match ::url::Url::parse(base) {
        Ok(parsed) => match parsed.join(path) {
            Ok(joined) => joined.to_string().trim_end_matches('/').to_string(),
            Err(_) => fallback_concat(base, path),
        },
        Err(_) => fallback_concat(base, path),
    }
}

/// Last-resort URL join used when `::url::Url::parse` rejects the base.
///
/// Guarantees a slash between `base` and `path` regardless of whether either
/// carries one, but does not otherwise validate the resulting string.
#[inline]
fn fallback_concat(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    if path.starts_with('/') {
        format!("{base}{path}")
    } else {
        format!("{base}/{path}")
    }
}

#[cfg(test)]
#[path = "url_tests.rs"]
mod tests;
