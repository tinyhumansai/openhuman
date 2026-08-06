//! Medulla sub-facade — typed access to the Medulla orchestration backend.
//!
//! Follows the shape [`super::config`] established: a borrowed newtype over the
//! runtime, and two-line methods delegating to [`call`](super::call::call).
//!
//! # Types are re-exported, not redefined
//!
//! [`MedullaStatus`], [`SessionSummary`] and [`RosterWorker`] come straight from
//! the domain rather than being mirrored here. A parallel set of facade structs
//! would be one more thing to keep in step with the wire contract, and the whole
//! point of the domain owning them is that there is a single definition.
//!
//! # Gating
//!
//! Compiled only with the `medulla` feature, like the domain it wraps. With the
//! feature off `Core::medulla()` does not exist, so a host that cannot use it
//! fails to compile against it rather than discovering an error at runtime.

use std::sync::Arc;

use super::call::call;
use super::error::CoreError;
use crate::core::runtime::CoreRuntime;

pub use crate::openhuman::medulla::client::{
    AbortResult, EventEnvelope, Message, RosterWorker, SendResult, SessionCreated, SessionDetail,
    SessionSummary,
};
pub use crate::openhuman::medulla::ops::MedullaStatus;

/// Typed access to the Medulla backend.
///
/// Obtained from [`Core::medulla`](super::Core::medulla); never constructed
/// directly.
pub struct Medulla<'a>(pub(super) &'a Arc<CoreRuntime>);

impl Medulla<'_> {
    /// Whether the integration is configured and signed in.
    ///
    /// Makes no network call and does not fail on an unconfigured host — the
    /// result carries a `configured` flag and a stable reason instead. A host
    /// polls this to decide whether to show the Medulla surface at all, so
    /// "not set up" has to be a value it can render, not an error it must
    /// special-case.
    pub async fn status(&self) -> Result<MedullaStatus, CoreError> {
        call(self.0, "openhuman.medulla_status", serde_json::json!({})).await
    }

    /// List the operator's durable sessions.
    ///
    /// # Errors
    ///
    /// [`CoreError::Domain`] with `kind = "MedullaNoBaseUrl"` or
    /// `"MedullaNoSessionToken"` when the integration is not usable, both
    /// flagged `expected_user_state` so a host renders a notice rather than a
    /// failure. Backend rejections carry the backend's own `errorCode` as
    /// `kind`, and HTTP 401/403 are likewise `expected_user_state`.
    pub async fn list_sessions(&self) -> Result<Vec<SessionSummary>, CoreError> {
        call(
            self.0,
            "openhuman.medulla_list_sessions",
            serde_json::json!({}),
        )
        .await
    }

    /// Create a durable session.
    ///
    /// `title` is optional — the backend names an untitled session itself
    /// rather than the host inventing one.
    pub async fn create_session(&self, title: Option<&str>) -> Result<SessionCreated, CoreError> {
        call(
            self.0,
            "openhuman.medulla_create_session",
            serde_json::json!({ "title": title }),
        )
        .await
    }

    /// Fetch one session's state.
    pub async fn get_session(&self, session_id: &str) -> Result<SessionDetail, CoreError> {
        call(
            self.0,
            "openhuman.medulla_get_session",
            serde_json::json!({ "sessionId": session_id }),
        )
        .await
    }

    /// Send a message to a session.
    ///
    /// `sync = false` returns as soon as the backend accepts the turn, leaving
    /// the reply to arrive over the event stream; `true` blocks until it
    /// replies. A UI wants the former so it can render progress, a scripted
    /// caller usually wants the latter.
    pub async fn send_message(
        &self,
        session_id: &str,
        body: &str,
        sync: bool,
    ) -> Result<SendResult, CoreError> {
        call(
            self.0,
            "openhuman.medulla_send_message",
            serde_json::json!({ "sessionId": session_id, "body": body, "sync": sync }),
        )
        .await
    }

    /// Abort a session's running cycle.
    pub async fn abort(&self, session_id: &str) -> Result<AbortResult, CoreError> {
        call(
            self.0,
            "openhuman.medulla_abort",
            serde_json::json!({ "sessionId": session_id }),
        )
        .await
    }

    /// Replay a session's messages after `after`.
    ///
    /// `after` is a cursor, not a page offset: passing the last seq already
    /// seen returns only what is new, which is what makes a reconnect cheap.
    pub async fn list_messages(
        &self,
        session_id: &str,
        after: Option<i64>,
    ) -> Result<Vec<Message>, CoreError> {
        call(
            self.0,
            "openhuman.medulla_list_messages",
            serde_json::json!({ "sessionId": session_id, "after": after }),
        )
        .await
    }

    /// Replay a session's events after `after`.
    ///
    /// Same cursor semantics as [`list_messages`](Self::list_messages).
    pub async fn list_events(
        &self,
        session_id: &str,
        after: Option<i64>,
    ) -> Result<Vec<EventEnvelope>, CoreError> {
        call(
            self.0,
            "openhuman.medulla_list_events",
            serde_json::json!({ "sessionId": session_id, "after": after }),
        )
        .await
    }

    /// Read the roster of workers currently connected to the backend.
    ///
    /// # Errors
    ///
    /// Same shape as [`list_sessions`](Self::list_sessions).
    pub async fn roster(&self) -> Result<Vec<RosterWorker>, CoreError> {
        call(self.0, "openhuman.medulla_roster", serde_json::json!({})).await
    }
}

#[cfg(test)]
mod tests {
    use crate::openhuman::medulla::all_medulla_registered_controllers;

    /// Every method this facade dispatches must name a registered controller.
    ///
    /// The facade's method names are strings; a typo or a renamed controller
    /// would otherwise surface as `CoreError::Unavailable` at runtime, which is
    /// indistinguishable from a domain the host gated off on purpose. Pinning
    /// them here turns that into a test failure.
    #[test]
    fn every_dispatched_method_is_registered() {
        let registered: Vec<String> = all_medulla_registered_controllers()
            .iter()
            .map(|c| c.rpc_method_name())
            .collect();

        for method in [
            "openhuman.medulla_status",
            "openhuman.medulla_list_sessions",
            "openhuman.medulla_create_session",
            "openhuman.medulla_get_session",
            "openhuman.medulla_send_message",
            "openhuman.medulla_abort",
            "openhuman.medulla_list_messages",
            "openhuman.medulla_list_events",
            "openhuman.medulla_roster",
        ] {
            assert!(
                registered.iter().any(|m| m == method),
                "facade dispatches `{method}`, which no controller registers. \
                 Registered: {registered:?}"
            );
        }
    }
}
