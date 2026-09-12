//! Business logic for the `medulla` RPC namespace.
//!
//! Each function resolves a client from ambient config, performs one backend
//! call, and returns an [`RpcOutcome`]. Transport and client-construction
//! failures become [`StructuredRpcError`]s so a host can branch on a stable
//! `data.kind` instead of matching on message text.

use serde::{Deserialize, Serialize};

use crate::openhuman::config::Config;
use crate::rpc::{RpcOutcome, StructuredRpcError};

use super::client::{
    AbortResult, ClientError, MedullaClient, Message, RosterWorker, SendResult, SessionCreated,
    SessionDetail, SessionSummary, WireEventEnvelope,
};
use super::resolve::{self, NotConfigured};

/// Whether the Medulla integration is usable, and why not when it isn't.
///
/// `Deserialize` as well as `Serialize` because this type round-trips: ops
/// serializes it onto the RPC boundary and the embed facade deserializes it
/// back on the other side. An output-only derive compiles until the facade
/// tries to read it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MedullaStatus {
    /// True when a base URL and a session token are both available.
    pub configured: bool,
    /// The resolved base URL, when one is configured.
    ///
    /// Safe to surface: it is an operator-supplied endpoint, never a credential.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Whether a session token was resolved. The token itself is never returned.
    pub has_session_token: bool,
    /// Stable reason discriminator when `configured` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Report integration readiness without making a network call.
///
/// Deliberately infallible: "not configured" is a state to render, not an error
/// to raise, and a host polls this to decide whether to show the surface at all.
pub async fn status(config: &Config) -> Result<RpcOutcome<MedullaStatus>, String> {
    let base = resolve::base_url(config);
    let status = match resolve::client(config) {
        Ok(_) => MedullaStatus {
            configured: true,
            base_url: base,
            has_session_token: true,
            reason: None,
        },
        Err(reason) => MedullaStatus {
            configured: false,
            has_session_token: !matches!(reason, NotConfigured::NoSessionToken),
            base_url: base,
            reason: Some(reason.kind().to_string()),
        },
    };
    log::debug!(
        "[medulla] status configured={} reason={:?}",
        status.configured,
        status.reason
    );
    Ok(RpcOutcome::new(status, Vec::new()))
}

/// List the operator's durable sessions.
pub async fn list_sessions(config: &Config) -> Result<RpcOutcome<Vec<SessionSummary>>, String> {
    let client = resolved(config)?;
    let sessions = call(client.list_sessions().await, "medulla_list_sessions")?;
    log::debug!("[medulla] list_sessions count={}", sessions.len());
    Ok(RpcOutcome::new(sessions, Vec::new()))
}

/// Create a durable session.
///
/// `title` is optional; the backend names an untitled session itself rather
/// than this host inventing one.
pub async fn create_session(
    config: &Config,
    title: Option<&str>,
) -> Result<RpcOutcome<SessionCreated>, String> {
    let client = resolved(config)?;
    let created = call(client.create_session(title).await, "medulla_create_session")?;
    log::debug!("[medulla] create_session id={}", created.session_id);
    Ok(RpcOutcome::new(created, Vec::new()))
}

/// Fetch one session's state.
pub async fn get_session(
    config: &Config,
    session_id: &str,
) -> Result<RpcOutcome<SessionDetail>, String> {
    let client = resolved(config)?;
    let detail = call(client.get_session(session_id).await, "medulla_get_session")?;
    Ok(RpcOutcome::new(detail, Vec::new()))
}

/// Send a message to a session.
///
/// `sync = false` returns as soon as the backend accepts the turn; `true`
/// blocks until it replies. The caller chooses, because a TUI wants the former
/// (so it can render streaming progress) while a scripted client wants the
/// latter.
pub async fn send_message(
    config: &Config,
    session_id: &str,
    body: &str,
    sync: bool,
) -> Result<RpcOutcome<SendResult>, String> {
    let client = resolved(config)?;
    let result = call(
        client.send_message(session_id, body, sync).await,
        "medulla_send_message",
    )?;
    log::debug!(
        "[medulla] send_message session={session_id} sync={sync} cycle={} seq={}",
        result.cycle_id,
        result.seq
    );
    Ok(RpcOutcome::new(result, Vec::new()))
}

/// Abort a session's running cycle.
pub async fn abort(config: &Config, session_id: &str) -> Result<RpcOutcome<AbortResult>, String> {
    let client = resolved(config)?;
    let result = call(client.abort(session_id).await, "medulla_abort")?;
    log::debug!(
        "[medulla] abort session={session_id} aborted={}",
        result.aborted
    );
    Ok(RpcOutcome::new(result, Vec::new()))
}

/// Replay a session's messages after `after`.
///
/// `after` is a cursor, not a page offset: passing the last seq already seen
/// returns only what is new, which is what makes a reconnect cheap.
pub async fn list_messages(
    config: &Config,
    session_id: &str,
    after: Option<i64>,
) -> Result<RpcOutcome<Vec<Message>>, String> {
    let client = resolved(config)?;
    let messages = call(
        client.list_messages(session_id, after).await,
        "medulla_list_messages",
    )?;
    log::debug!(
        "[medulla] list_messages session={session_id} after={after:?} count={}",
        messages.len()
    );
    Ok(RpcOutcome::new(messages, Vec::new()))
}

/// Replay a session's events after `after`.
///
/// Same cursor semantics as [`list_messages`].
pub async fn list_events(
    config: &Config,
    session_id: &str,
    after: Option<i64>,
) -> Result<RpcOutcome<Vec<WireEventEnvelope>>, String> {
    let client = resolved(config)?;
    let events = call(
        client.list_events(session_id, after).await,
        "medulla_list_events",
    )?;
    log::debug!(
        "[medulla] list_events session={session_id} after={after:?} count={}",
        events.len()
    );
    Ok(RpcOutcome::new(events, Vec::new()))
}

/// Read the connected worker roster.
pub async fn roster(config: &Config) -> Result<RpcOutcome<Vec<RosterWorker>>, String> {
    let client = resolved(config)?;
    let workers = call(client.roster().await, "medulla_roster")?;
    log::debug!("[medulla] roster count={}", workers.len());
    Ok(RpcOutcome::new(workers, Vec::new()))
}

/// Resolve a client, mapping "not configured" to a structured error.
///
/// Both reasons are flagged `expected_user_state`: a signed-out or unconfigured
/// host is a state the operator can fix, not an internal fault, so the RPC
/// boundary suppresses Sentry for them.
fn resolved(config: &Config) -> Result<MedullaClient, String> {
    resolve::client(config).map_err(|reason| {
        StructuredRpcError {
            message: reason.message().to_string(),
            data: Some(serde_json::json!({ "kind": reason.kind() })),
            expected_user_state: true,
        }
        .encode()
    })
}

/// Map a client error into a structured RPC error.
///
/// The backend's own `errorCode` becomes `data.kind` when present, so a host
/// branches on the backend's vocabulary rather than on prose. HTTP 401/403 are
/// marked `expected_user_state` — an expired session is a sign-in prompt, not a
/// bug report.
fn call<T>(result: Result<T, ClientError>, method: &'static str) -> Result<T, String> {
    result.map_err(|err| {
        let (kind, status, expected) = match &err {
            ClientError::Api {
                status, error_code, ..
            } => {
                let expected = matches!(status, Some(401) | Some(403));
                (error_code.clone(), *status, expected)
            }
            _ => (None, None, false),
        };
        log::debug!("[medulla] rpc_failed method={method} status={status:?} kind={kind:?}");
        StructuredRpcError {
            message: err.to_string(),
            data: Some(serde_json::json!({
                "kind": kind.unwrap_or_else(|| "MedullaRequestFailed".to_string()),
                "status": status,
            })),
            expected_user_state: expected,
        }
        .encode()
    })
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
