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

use crate::api::config::effective_backend_api_url;
use crate::api::jwt::get_session_token;
use crate::api::BackendOAuthClient;
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

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
fn ensure_secure_backend_url(api_url: &str) -> Result<(), String> {
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
/// Follows the `reply_speech::synthesize_reply` auth pattern: session token →
/// [`BackendOAuthClient`] → `authed_json`, with `flatten_authed_error` so a
/// lapsed-session 401 classifies as `SESSION_EXPIRED` and skips Sentry rather
/// than leaking as a raw error string.
pub async fn mint_voice_agent_signed_url(
    config: &Config,
) -> Result<RpcOutcome<VoiceAgentSignedUrl>, String> {
    let token = get_session_token(config)
        .map_err(|e| e.to_string())?
        .and_then(|t| {
            let s = t.trim().to_string();
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        })
        .ok_or_else(|| "no backend session token; sign in first".to_string())?;

    let api_url = effective_backend_api_url(&config.api_url);
    ensure_secure_backend_url(&api_url)?;
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;

    let raw = client
        .authed_json(&token, Method::GET, "/voice-agent/get-signed-url", None)
        .await
        .map_err(crate::api::flatten_authed_error)?;

    let result = parse_signed_url_response(&raw)?;
    debug!("{LOG_PREFIX} minted signed url agent={}", result.agent_id);
    Ok(RpcOutcome::single_log(
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
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_the_wrapped_success_envelope() {
        let raw = json!({ "success": true, "data": { "signedUrl": "wss://x", "agentId": "a1", "userToken": "tok" } });
        let out = parse_signed_url_response(&raw).unwrap();
        assert_eq!(out.signed_url, "wss://x");
        assert_eq!(out.agent_id, "a1");
        assert_eq!(out.user_token, "tok");
    }

    #[test]
    fn tolerates_a_bare_object() {
        let raw = json!({ "signedUrl": "wss://y", "agentId": "a2" });
        let out = parse_signed_url_response(&raw).unwrap();
        assert_eq!(out.signed_url, "wss://y");
        assert_eq!(out.agent_id, "a2");
    }

    #[test]
    fn user_token_defaults_empty_against_an_older_backend() {
        let raw = json!({ "data": { "signedUrl": "wss://y", "agentId": "a2" } });
        let out = parse_signed_url_response(&raw).unwrap();
        assert_eq!(out.user_token, "");
    }

    #[test]
    fn secure_url_guard_allows_https_and_loopback_http_only() {
        assert!(ensure_secure_backend_url("https://api.tinyhumans.ai").is_ok());
        assert!(ensure_secure_backend_url("http://localhost:5005").is_ok());
        assert!(ensure_secure_backend_url("http://127.0.0.1:5005/").is_ok());
        let err = ensure_secure_backend_url("http://api.tinyhumans.ai").unwrap_err();
        assert!(err.contains("non-HTTPS"), "{err}");
    }

    #[test]
    fn secure_url_guard_allows_ipv6_loopback() {
        // Bracketed IPv6 loopback must be accepted — the previous first-`:`
        // split turned `[::1]:5005` into `"["` and wrongly rejected it.
        assert!(ensure_secure_backend_url("http://[::1]:5005").is_ok());
        assert!(ensure_secure_backend_url("http://[::1]:5005/").is_ok());
        assert!(ensure_secure_backend_url("http://[::1]").is_ok());
        // A non-loopback bracketed IPv6 host is still rejected over http://.
        let err = ensure_secure_backend_url("http://[2001:db8::1]:5005").unwrap_err();
        assert!(err.contains("non-HTTPS"), "{err}");
    }

    #[test]
    fn errors_when_signed_url_is_absent() {
        let raw = json!({ "data": { "agentId": "a3" } });
        let err = parse_signed_url_response(&raw).unwrap_err();
        assert!(err.contains("no signed_url"), "{err}");
    }

    #[test]
    fn result_serializes_snake_case_for_the_wire() {
        let json = serde_json::to_value(VoiceAgentSignedUrl {
            signed_url: "wss://z".into(),
            agent_id: "a4".into(),
            user_token: "tok".into(),
        })
        .unwrap();
        assert_eq!(json.get("signed_url").unwrap(), "wss://z");
        assert_eq!(json.get("agent_id").unwrap(), "a4");
        assert_eq!(json.get("user_token").unwrap(), "tok");
    }
}
