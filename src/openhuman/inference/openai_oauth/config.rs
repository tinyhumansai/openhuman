//! OpenAI Codex (ChatGPT subscription) OAuth endpoints and client registration.

use motosan_ai_oauth::providers::codex::codex;
use motosan_ai_oauth::OAuthConfig;

/// Loopback redirect registered with the Codex public OAuth app.
pub const REDIRECT_URI: &str = "http://127.0.0.1:1455/auth/callback";

pub fn codex_oauth_config() -> OAuthConfig {
    codex()
}
