//! URL helpers shared across domains: base-URL normalisation, safe path
//! joining. Nothing here knows about any
//! particular backend — hosted-backend URL resolution lives with the host that
//! installs the backend transport (`openhuman-tinyhumans`).

/// Trim whitespace and strip trailing slashes so all base URLs are in
/// canonical form before being joined with a path.
///
/// This is deliberately a cheap string operation (no URL parsing) so it can
/// be called on potentially-invalid strings without panicking.
pub fn normalize_api_base_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

/// Like [`normalize_api_base_url`] but also **strips any inference-style path**
/// (e.g. `/openai/v1/chat/completions`) so the result is always a bare host
/// root suitable as a backend base.
///
/// # Why this exists
///
/// Users (and CI configs) sometimes set `BACKEND_URL` or `config.api_url` to
/// the full inference endpoint. Backend callers append domain-specific paths
/// (`/auth/me`, `/agent-integrations/…`) which then land on
/// `.../openai/v1/chat/completions/auth/me` — an obvious 404.
///
/// # Scheme-less fallback
///
/// `option_env!`-baked values occasionally omit the scheme
/// (e.g. `api.tinyhumans.ai/openai/v1/chat/completions`). We retry with an
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
/// | `https://api.tinyhumans.ai`               | `/auth/me`                | `https://api.tinyhumans.ai/auth/me`                                   |
/// | `https://api.tinyhumans.ai/openai/v1/…`   | `/agent-integrations/foo` | `https://api.tinyhumans.ai/agent-integrations/foo`  ← path replaced   |
/// | `https://api.tinyhumans.ai`               | `""`                      | `https://api.tinyhumans.ai`                                           |
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
