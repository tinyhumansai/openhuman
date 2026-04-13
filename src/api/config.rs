//! Base URL and defaults for the TinyHumans / AlphaHuman hosted API.

/// Default API host when `config.api_url` is unset or blank and no env override is set.
pub const DEFAULT_API_BASE_URL: &str = "https://api.tinyhumans.ai";

/// Resolves the hosted API base URL (no path suffix).
///
/// Order: non-empty `api_url` from config → [`api_base_from_env`] (`BACKEND_URL`, then `VITE_BACKEND_URL`)
/// → [`DEFAULT_API_BASE_URL`] (same host your curl command used when config is blank).
pub fn effective_api_url(api_url: &Option<String>) -> String {
    if let Some(u) = api_url.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        return normalize_api_base_url(u);
    }
    if let Some(env_url) = api_base_from_env() {
        return env_url;
    }
    DEFAULT_API_BASE_URL.to_string()
}

/// Trim and strip trailing slashes so paths join consistently.
pub fn normalize_api_base_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

/// Resolve API base from process environment (`BACKEND_URL` first, then `VITE_BACKEND_URL`).
pub fn api_base_from_env() -> Option<String> {
    std::env::var("BACKEND_URL")
        .or_else(|_| std::env::var("VITE_BACKEND_URL"))
        .ok()
        .map(|s| normalize_api_base_url(&s))
        .filter(|s| !s.is_empty())
}
