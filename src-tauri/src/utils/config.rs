/// Configuration constants and environment utilities
///
/// This module provides configuration values that can be
/// overridden via environment variables at runtime.
use std::env;

/// Default backend URL (can be overridden via BACKEND_URL env var)
pub const DEFAULT_BACKEND_URL: &str = "https://api.tinyhumans.ai";

/// Application identifier for keychain storage
pub const APP_IDENTIFIER: &str = "com.tinyhumansai.openhuman";

/// Service name for keychain
pub const KEYCHAIN_SERVICE: &str = "OpenHuman";

/// Get the backend URL from environment or use default
/// Checks VITE_BACKEND_URL first, then BACKEND_URL, then defaults
pub fn get_backend_url() -> String {
    let url = env::var("VITE_BACKEND_URL")
        .or_else(|_| env::var("BACKEND_URL"))
        .unwrap_or_else(|_| DEFAULT_BACKEND_URL.to_string());

    log::debug!("[config] Backend URL: {}", url);
    url
}
