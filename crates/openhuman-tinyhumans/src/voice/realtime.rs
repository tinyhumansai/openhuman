//! Realtime voice-agent session bootstrap — mints a short-lived signed
//! WebSocket URL from the hosted backend's `/voice-agent/get-signed-url`
//! endpoint so the desktop client can open an ElevenLabs Agents
//! (Conversational AI) session directly. The provider API key stays
//! server-side; the client only ever sees the signed URL (#5399).
//!
//! Approval gate (#1339) classification: **internal** — the user's own
//! assistant listening/speaking through the user's own mic/speakers, with no
//! third-party outbound effect.

use log::debug;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::core::Outcome;
use openhuman_embed::__host::security::credentials::session_support::resolve_backend_credential;

use crate::client::{flatten_authed_error, BackendClient};

const LOG_PREFIX: &str = "[voice-realtime]";

/// A short-lived signed URL for a realtime voice-agent session plus the agent
/// it was minted for. Mirrors the backend `/voice-agent/get-signed-url` shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceAgentSignedUrl {
    pub signed_url: String,
    pub agent_id: String,
    /// Short-lived token binding this session to the signed-in user. The
    /// renderer echoes it back as the ElevenLabs `userId`, which the backend
    /// relay verifies — so the Custom-LLM relay never trusts a raw user id
    /// (#5399). Empty only against an older backend that predates the binding.
    #[serde(default)]
    pub user_token: String,
}

/// Reject a plaintext backend URL for this credentialed request: the session
/// token is a bearer credential, so it must not travel over `http://` to a
/// remote host (CWE-319). Loopback stays allowed so local-backend development
/// (`http://localhost:5005`) still works.
pub(crate) fn ensure_secure_backend_url(api_url: &str) -> Result<(), String> {
    let lowered = api_url.trim().to_ascii_lowercase();
    if lowered.starts_with("https://") {
        return Ok(());
    }
    let is_loopback = lowered
        .strip_prefix("http://")
        .map(authority_host)
        .map(|host| matches!(host, "localhost" | "127.0.0.1" | "::1"))
        .unwrap_or(false);
    if is_loopback {
        return Ok(());
    }
    Err(format!(
        "refusing to send the session token to a non-HTTPS backend ({api_url}); \
         set a https:// api_url"
    ))
}

/// Extract the host from an `http://`-stripped remainder, preserving a bracketed
/// IPv6 literal. Splitting on the first `:` (as the previous impl did) turned
/// `[::1]:5005` into `"["`, so the documented IPv6 loopback backend could never
/// match. Parse the authority structurally instead: `[::1]:5005` → `::1`,
/// `127.0.0.1:5005` → `127.0.0.1`, `localhost/path` → `localhost`.
fn authority_host(rest: &str) -> &str {
    // The authority ends at the first path separator.
    let authority = rest.split('/').next().unwrap_or("");
    if let Some(after_bracket) = authority.strip_prefix('[') {
        // Bracketed IPv6 literal: the host sits between '[' and ']'.
        return after_bracket.split(']').next().unwrap_or("");
    }
    // Otherwise the host is everything before the optional `:port`.
    authority.split(':').next().unwrap_or("")
}

/// Mint a realtime voice-agent signed URL by proxying the hosted backend.
///
/// Follows the `reply_speech::synthesize_reply` auth pattern: backend credential →
/// [`BackendClient`] → `authed_json`, with `flatten_authed_error` so a
/// lapsed-session 401 classifies as `SESSION_EXPIRED` and skips Sentry rather
/// than leaking as a raw error string.
pub async fn mint_voice_agent_signed_url(
    config: &Config,
) -> Result<Outcome<VoiceAgentSignedUrl>, String> {
    // API key (sent as `x-api-key`) or live session JWT (sent as Bearer).
    let credential = resolve_backend_credential(config)?;

    let client = BackendClient::from_config(config).map_err(|e| e.to_string())?;
    ensure_secure_backend_url(client.base_url())?;

    let raw = client
        .authed_json(
            &credential,
            Method::GET,
            "/voice-agent/get-signed-url",
            None,
        )
        .await
        .map_err(flatten_authed_error)?;

    let result = parse_signed_url_response(&raw)?;
    debug!("{LOG_PREFIX} minted signed url agent={}", result.agent_id);
    Ok(Outcome::single_log(
        result,
        "voice agent signed url minted via GET /voice-agent/get-signed-url",
    ))
}

/// Translate the backend's `{ success, data: { signedUrl, agentId } }` envelope
/// (or a bare object) into the UI contract. Kept separate so the parsing is
/// unit-testable without a live backend.
fn parse_signed_url_response(raw: &Value) -> Result<VoiceAgentSignedUrl, String> {
    let data = raw.get("data").unwrap_or(raw);
    let signed_url = data
        .get("signedUrl")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let agent_id = data
        .get("agentId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let user_token = data
        .get("userToken")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    if signed_url.is_empty() {
        return Err("backend returned no signed_url for the voice agent".to_string());
    }
    Ok(VoiceAgentSignedUrl {
        signed_url,
        agent_id,
        user_token,
    })
}

#[cfg(test)]
#[path = "realtime_tests.rs"]
mod tests;
