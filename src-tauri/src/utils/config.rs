/// Configuration constants and environment utilities
///
/// This module provides configuration values that can be
/// overridden via environment variables at runtime.

use std::env;

/// Default backend URL (can be overridden via BACKEND_URL env var)
pub const DEFAULT_BACKEND_URL: &str = "https://api.alphahuman.io";

/// Application identifier for keychain storage
pub const APP_IDENTIFIER: &str = "com.megamind.tauri-app";

/// Service name for keychain
pub const KEYCHAIN_SERVICE: &str = "AlphaHuman";

/// Deep link scheme
pub const DEEP_LINK_SCHEME: &str = "alphahuman";

/// Socket.io reconnection settings
pub const SOCKET_RECONNECT_ATTEMPTS: u32 = 5;
pub const SOCKET_RECONNECT_DELAY_MS: u64 = 1000;
pub const SOCKET_PING_INTERVAL_MS: u64 = 25000;

/// Get the backend URL from environment or use default
pub fn get_backend_url() -> String {
    env::var("BACKEND_URL").unwrap_or_else(|_| DEFAULT_BACKEND_URL.to_string())
}

/// Get the Telegram widget auth URL
pub fn get_telegram_widget_url() -> String {
    format!("{}/auth/telegram-widget?redirect={}://auth", get_backend_url(), DEEP_LINK_SCHEME)
}

/// Get the token exchange endpoint URL
pub fn get_token_exchange_url() -> String {
    format!("{}/auth/desktop-exchange", get_backend_url())
}
