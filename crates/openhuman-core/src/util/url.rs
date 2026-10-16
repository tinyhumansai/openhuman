//! URL helpers shared across domains: base-URL normalisation, safe path
//! joining and local-host classification. Nothing here knows about any
//! particular backend — hosted-backend URL resolution lives with the host that
//! registers the config enricher (`openhuman-tinyhumans`).

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

/// Returns `true` when the parsed URL's host is loopback, unspecified
/// (`0.0.0.0` / `[::]`), a private RFC 1918 IPv4 range, or `localhost`.
///
/// Using typed-host matching (via `url::Host` variants) rather than
/// `host_str()` string comparison ensures that IPv4-mapped IPv6 addresses
/// (`::ffff:127.0.0.1`), the bare IPv6 loopback (`::1`), and all three
/// IPv4 loopback forms classify correctly.
#[inline]
pub fn host_is_local(parsed: &::url::Url) -> bool {
    match parsed.host() {
        Some(::url::Host::Ipv4(addr)) => {
            addr.is_loopback() || addr.is_unspecified() || addr.is_private()
        }
        Some(::url::Host::Ipv6(addr)) => addr.is_loopback() || addr.is_unspecified(),
        Some(::url::Host::Domain(name)) => {
            let h = name.to_ascii_lowercase();
            h == "localhost" || h.ends_with(".localhost")
        }
        None => false,
    }
}

#[cfg(test)]
#[path = "url_tests.rs"]
mod tests;
