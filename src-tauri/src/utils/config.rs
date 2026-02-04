/// Configuration constants and environment utilities
///
/// This module provides configuration values that can be
/// overridden via environment variables at runtime.
use std::env;
use std::path::PathBuf;
use std::sync::Once;

/// Default backend URL (can be overridden via BACKEND_URL env var)
pub const DEFAULT_BACKEND_URL: &str = "https://api.alphahuman.xyz";

/// Application identifier for keychain storage
pub const APP_IDENTIFIER: &str = "com.alphahuman.app";

/// Service name for keychain
pub const KEYCHAIN_SERVICE: &str = "AlphaHuman";

/// Ensure .env is loaded once
static DOTENV_INIT: Once = Once::new();

/// Try to find and load .env file from various locations
fn ensure_dotenv_loaded() {
    DOTENV_INIT.call_once(|| {
        // Try current directory first (project root when running `tauri dev`)
        if dotenvy::dotenv().is_ok() {
            log::debug!("[config] Loaded .env from current directory");
            return;
        }

        // Try parent directory (when cwd is src-tauri)
        if let Ok(cwd) = env::current_dir() {
            let parent_env = cwd.parent().map(|p| p.join(".env"));
            if let Some(path) = parent_env {
                if path.exists() {
                    if dotenvy::from_path(&path).is_ok() {
                        log::debug!("[config] Loaded .env from parent directory: {:?}", path);
                        return;
                    }
                }
            }
        }

        // Try CARGO_MANIFEST_DIR (available during development builds)
        if let Ok(manifest_dir) = env::var("CARGO_MANIFEST_DIR") {
            let manifest_path = PathBuf::from(&manifest_dir);
            // .env is one level up from src-tauri
            let project_root_env = manifest_path.parent().map(|p| p.join(".env"));
            if let Some(path) = project_root_env {
                if path.exists() {
                    if dotenvy::from_path(&path).is_ok() {
                        log::debug!("[config] Loaded .env from project root: {:?}", path);
                        return;
                    }
                }
            }
        }

        log::debug!("[config] No .env file found, using defaults/environment");
    });
}

/// Get the backend URL from environment or use default
/// Checks VITE_BACKEND_URL first, then BACKEND_URL, then defaults
pub fn get_backend_url() -> String {
    ensure_dotenv_loaded();

    let url = env::var("VITE_BACKEND_URL")
        .or_else(|_| env::var("BACKEND_URL"))
        .unwrap_or_else(|_| DEFAULT_BACKEND_URL.to_string());

    log::debug!("[config] Backend URL: {}", url);
    url
}
