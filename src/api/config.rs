//! Base URL and defaults for the TinyHumans / AlphaHuman hosted API.

/// Default API host when `config.api_url` is unset or blank and no env override is set.
pub const DEFAULT_API_BASE_URL: &str = "https://api.tinyhumans.ai";
/// Default staging API host when the app environment is explicitly `staging`.
pub const DEFAULT_STAGING_API_BASE_URL: &str = "https://staging-api.tinyhumans.ai";
/// Primary app-environment selector used by the core and desktop app.
pub const APP_ENV_VAR: &str = "OPENHUMAN_APP_ENV";
/// Vite-exposed app-environment selector used by the frontend bundle.
pub const VITE_APP_ENV_VAR: &str = "VITE_OPENHUMAN_APP_ENV";

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
    default_api_base_url_for_env(app_env_from_env().as_deref()).to_string()
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

/// Resolve the app environment from process environment.
pub fn app_env_from_env() -> Option<String> {
    std::env::var(APP_ENV_VAR)
        .or_else(|_| std::env::var(VITE_APP_ENV_VAR))
        .ok()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
}

pub fn is_staging_app_env(app_env: Option<&str>) -> bool {
    matches!(app_env.map(str::trim), Some(env) if env.eq_ignore_ascii_case("staging"))
}

pub fn default_api_base_url_for_env(app_env: Option<&str>) -> &'static str {
    if is_staging_app_env(app_env) {
        DEFAULT_STAGING_API_BASE_URL
    } else {
        DEFAULT_API_BASE_URL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_app_env_uses_staging_default_api() {
        assert_eq!(
            default_api_base_url_for_env(Some("staging")),
            DEFAULT_STAGING_API_BASE_URL
        );
        assert!(is_staging_app_env(Some("STAGING")));
    }

    #[test]
    fn non_staging_app_env_uses_production_default_api() {
        assert_eq!(
            default_api_base_url_for_env(Some("production")),
            DEFAULT_API_BASE_URL
        );
        assert_eq!(default_api_base_url_for_env(None), DEFAULT_API_BASE_URL);
        assert!(!is_staging_app_env(Some("development")));
    }
}
