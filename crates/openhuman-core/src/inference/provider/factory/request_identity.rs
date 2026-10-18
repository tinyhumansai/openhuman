//! What a BYOK cloud request says about who is calling: the product
//! `User-Agent` every OpenAI-compatible route carries, and the per-conversation
//! session id that OpenCode Go requires (#7865).
//!
//! OpenCode Go (`https://opencode.ai/zen/go/v1`) rejects every chat request
//! that lacks an `x-opencode-session` header with HTTP 400 `MissingSessionID`,
//! and asks clients to identify themselves with a product user agent rather
//! than an HTTP library's. The id only has to be opaque and stable for one
//! conversation — it drives prompt-cache routing on their side — so it is
//! derived from the turn origin the factory already runs under: the chat
//! thread, the channel conversation, or the automation job. Auxiliary calls
//! made for the same thread (title generation, summarization) resolve the same
//! origin and therefore the same id.

use std::sync::LazyLock;

use sha2::{Digest, Sha256};

use crate::agent::turn_origin::AgentTurnOrigin;
use crate::config::schema::cloud_providers::endpoint_host;

/// The header OpenCode Go routes conversations by.
pub const OPENCODE_SESSION_HEADER: &str = "x-opencode-session";

/// The `User-Agent` OpenHuman presents to OpenAI-compatible cloud providers
/// (`openhuman/<core version>`), unless a route sets its own (Codex OAuth) or
/// the provider entry configures one.
pub fn product_user_agent() -> String {
    format!("openhuman/{}", env!("CARGO_PKG_VERSION"))
}

/// Whether `endpoint` is an OpenCode Go base URL, i.e. one that refuses
/// requests without [`OPENCODE_SESSION_HEADER`]. Matched on the `opencode.ai`
/// host and the `/zen/go` path prefix so the regular Zen endpoint, which has
/// no such requirement, is left alone.
pub fn endpoint_requires_opencode_session(endpoint: &str) -> bool {
    if endpoint_host(endpoint).as_deref() != Some("opencode.ai") {
        return false;
    }
    let trimmed = endpoint.trim();
    let after_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    let path = after_scheme
        .find('/')
        .map(|i| &after_scheme[i..])
        .unwrap_or("");
    let path = path.to_ascii_lowercase();
    path == "/zen/go" || path.starts_with("/zen/go/")
}

/// The conversation a turn origin belongs to, as a key that is the same for
/// every request made on behalf of that conversation and different across
/// conversations. `None` for origins that carry no conversation (CLI, direct
/// chat, unknown), which fall back to one id per process.
pub fn conversation_key(origin: Option<&AgentTurnOrigin>) -> Option<String> {
    match origin? {
        AgentTurnOrigin::WebChat { thread_id, .. } => Some(format!("thread:{thread_id}")),
        AgentTurnOrigin::ExternalChannel {
            channel,
            history_key,
            reply_target,
            ..
        } => Some(format!(
            "channel:{channel}:{}",
            history_key.as_deref().unwrap_or(reply_target)
        )),
        AgentTurnOrigin::TrustedAutomation { job_id, .. } => Some(format!("job:{job_id}")),
        _ => None,
    }
}

/// A stable id for the process, used when the origin names no conversation
/// so the requests of one CLI run still share a session.
static PROCESS_CONVERSATION_KEY: LazyLock<String> =
    LazyLock::new(|| format!("process:{}", uuid::Uuid::new_v4()));

/// The `x-opencode-session` value for `origin`: a 32-hex-character digest of
/// the conversation key, so a thread id never leaves the machine verbatim.
pub fn opencode_session_id(origin: Option<&AgentTurnOrigin>) -> String {
    let key = conversation_key(origin).unwrap_or_else(|| PROCESS_CONVERSATION_KEY.clone());
    let mut hasher = Sha256::new();
    hasher.update(b"openhuman-opencode-session-v1:");
    hasher.update(key.as_bytes());
    let digest = hasher.finalize();
    digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Whether `headers` already carries `name` (case-insensitive), so a value the
/// user configured on the provider entry wins over the built-in one.
pub fn has_header(headers: &[(String, String)], name: &str) -> bool {
    headers
        .iter()
        .any(|(existing, _)| existing.eq_ignore_ascii_case(name))
}

/// Completes a cloud route's request identity in place: the product
/// `User-Agent` when the route set none and the entry configured none, and the
/// OpenCode Go session header when `endpoint` requires it and the entry did
/// not configure one. `origin` is the turn the client is built for.
pub fn apply_request_identity(
    endpoint: &str,
    origin: Option<&AgentTurnOrigin>,
    headers: &mut Vec<(String, String)>,
    user_agent: &mut Option<String>,
) {
    if user_agent.is_none() && !has_header(headers, "user-agent") {
        *user_agent = Some(product_user_agent());
    }
    if endpoint_requires_opencode_session(endpoint) && !has_header(headers, OPENCODE_SESSION_HEADER)
    {
        headers.push((
            OPENCODE_SESSION_HEADER.to_string(),
            opencode_session_id(origin),
        ));
    }
}

#[cfg(test)]
#[path = "request_identity_tests.rs"]
mod tests;
