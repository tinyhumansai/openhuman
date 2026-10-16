//! [`HostedApiError`]: the typed, expected backend states `authed_json`
//! surfaces for callers to recover from in-flow, and
//! [`flatten_authed_error`], the chokepoint that turns them into the
//! sentinel strings `core::observability` classifies.

use openhuman_embed::__host::core::observability::{
    API_KEY_REJECTED_PREFIX, BACKEND_UNAVAILABLE_PREFIX,
};

/// Typed errors surfaced by `BackendClient::authed_json` for expected backend
/// states that callers should recover from rather than funnel into Sentry.
#[derive(Debug, thiserror::Error)]
pub enum HostedApiError {
    /// Edit / delete of a channel message returned 404: the user deleted the
    /// message on the provider side (Telegram, Discord, Slack, ...) but local
    /// streaming state still has the id, or the backend GC'd the relay row.
    /// Callers clear stale state and skip the retry.
    #[error("message not found on {provider}: {message_id}")]
    MessageNotFound {
        /// Channel provider segment (e.g. `"telegram"`, `"discord"`).
        provider: String,
        /// Provider-specific message id from the URL.
        message_id: String,
    },
    /// The backend rejected the bearer JWT with `401`. An expected
    /// user-session state (expired, revoked, rotated), not a bug; the auth
    /// domain owns recovery.
    #[error("backend rejected session token on {method} {path}")]
    Unauthorized {
        /// HTTP method (`"GET"`, `"POST"`, ...).
        method: String,
        /// Request path the 401 came back from (no query string).
        path: String,
    },
    /// The backend rejected a TinyHumans API key (`x-api-key`) with `401`.
    ///
    /// Distinct from [`Self::Unauthorized`] on purpose: that variant flattens
    /// onto the `SESSION_EXPIRED` sentinel, which makes the dispatcher clear
    /// the app session and sign out. A rejected key on a runtime that never
    /// had a session must not trigger that recovery.
    #[error("backend rejected api key on {method} {path}")]
    ApiKeyRejected {
        /// HTTP method (`"GET"`, `"POST"`, ...).
        method: String,
        /// Request path the 401 came back from (no query string).
        path: String,
    },
    /// `PATCH /channels/<provider>/messages/<id>` returned 404 because the
    /// backend **implements no such route** (#5230), not because the message
    /// is gone. Callers keep the message id and only disable the edit
    /// capability. Must stay distinct from [`Self::MessageNotFound`].
    #[error(
        "channel message edit route not implemented by backend ({provider}, message {message_id})"
    )]
    ChannelEditUnsupported {
        /// Channel provider segment (e.g. `"telegram"`, `"discord"`).
        provider: String,
        /// Provider-specific message id from the URL.
        message_id: String,
    },
    /// The request could not be sent at all because no transport could be
    /// built. An expected build condition, never a bug; flattens onto the
    /// `BACKEND_UNAVAILABLE:` sentinel.
    #[error("backend transport not available for {method} {path}")]
    BackendUnavailable {
        /// HTTP method (`"GET"`, `"POST"`, ...).
        method: String,
        /// Request path that could not be sent (no query string).
        path: String,
    },
}

/// Flatten an `authed_json` error onto the JSON-RPC `String` channel.
///
/// `Unauthorized` maps onto the existing `SESSION_EXPIRED` sentinel so the
/// dispatcher skips Sentry and publishes `DomainEvent::SessionExpired` (#3297).
/// `ApiKeyRejected` and `BackendUnavailable` map onto their own core prefixes.
/// This keys off the typed downcast, not the Display wording. Every other
/// error keeps its full `{e:#}` chain so genuine failures still reach Sentry.
pub fn flatten_authed_error(err: anyhow::Error) -> String {
    match err.downcast_ref::<HostedApiError>() {
        Some(HostedApiError::Unauthorized { method, path }) => {
            format!("SESSION_EXPIRED: backend rejected session token on {method} {path}")
        }
        Some(HostedApiError::ApiKeyRejected { method, path }) => {
            format!("{API_KEY_REJECTED_PREFIX}backend rejected api key on {method} {path}")
        }
        Some(HostedApiError::BackendUnavailable { method, path }) => {
            format!(
                "{BACKEND_UNAVAILABLE_PREFIX}backend transport not available for {method} {path}"
            )
        }
        _ => format!("{err:#}"),
    }
}
