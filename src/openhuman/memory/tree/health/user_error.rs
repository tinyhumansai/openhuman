//! The wire payload for the memory subsystem's `user_error` web-channel event.
//!
//! `tinymemory_core::tree::health::user_error` decides *that* a local-runtime
//! failure must reach the user; this module decides *what goes on the wire*.
//! The split is the same one the rest of the extraction follows — web channels,
//! and the socket the payload rides, are host surface.
//!
//! Both halves key on `LOCAL_MODEL_UNAVAILABLE_KIND`, which lives in the
//! contract crate so the two cannot drift apart silently.

use tinymemory_api::host::{LOCAL_MODEL_UNAVAILABLE_KIND, MEMORY_USER_ERROR_SOURCE};

use crate::core::socketio::WebChannelEvent;

/// The metadata-only `user_error` payload for an unusable local embedding
/// runtime. Built separately from the publish so the no-leak contract is
/// unit-testable without a live socket.
///
/// Metadata only, exactly like the cron producer: a stable `kind` token in
/// `error_type` plus `error_source`, and never the raw provider text, the model
/// id, or the configured endpoint (which can carry a private host).
pub(crate) fn local_model_unavailable_user_error() -> WebChannelEvent {
    WebChannelEvent {
        event: "user_error".to_string(),
        // Every socket auto-joins the "system" room, so this reaches all
        // connected clients rather than one chat session.
        client_id: "system".to_string(),
        error_type: Some(LOCAL_MODEL_UNAVAILABLE_KIND.to_string()),
        error_source: Some(MEMORY_USER_ERROR_SOURCE.to_string()),
        ..Default::default()
    }
}

/// Broadcast the local-runtime user error to every connected client.
///
/// Called from the `MemoryEvent::LocalModelUnavailable` arm of the sink in
/// [`crate::openhuman::memory::host`]. `origin` names the producer
/// (`health_gate` / `embed_classify`) and is logged, never sent.
pub(crate) fn publish_local_model_unavailable_user_error(origin: &str) {
    log::debug!(
        "[memory::host] action=broadcast_user_error kind={LOCAL_MODEL_UNAVAILABLE_KIND} \
         source={MEMORY_USER_ERROR_SOURCE} origin={origin}"
    );
    crate::openhuman::web_chat::publish_web_channel_event(local_model_unavailable_user_error());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the wire shape the frontend `socketService` handler reads, plus the
    /// metadata-only no-leak contract.
    #[test]
    fn payload_is_metadata_only() {
        let event = local_model_unavailable_user_error();

        assert_eq!(event.event, "user_error");
        // The "system" room is the one every socket auto-joins.
        assert_eq!(event.client_id, "system");
        assert_eq!(
            event.error_type.as_deref(),
            Some(LOCAL_MODEL_UNAVAILABLE_KIND)
        );
        assert_eq!(
            event.error_source.as_deref(),
            Some(MEMORY_USER_ERROR_SOURCE)
        );

        // Nothing that could carry the base URL, a model id, or raw provider
        // prose may ride along.
        assert!(event.message.is_none(), "must not carry raw error prose");
        assert!(event.full_response.is_none());
        assert!(event.thread_id.is_empty());
    }

    /// The kind token is a cross-language contract: `app/src/types/userError.ts`
    /// declares this exact `UserErrorKind` discriminator and `classify.ts` keys
    /// on it. A rename on either side drops the signal with no compile error on
    /// either side, so pin the wire string.
    #[test]
    fn kind_matches_frontend_discriminator() {
        assert_eq!(LOCAL_MODEL_UNAVAILABLE_KIND, "local_model_unavailable");
    }

    /// `socketService` only maps `error_source == "memory"` onto the `memory`
    /// scope; anything else falls back to the historical `cron` default, which
    /// would file this entry under the wrong heading.
    #[test]
    fn source_matches_frontend_scope_mapping() {
        assert_eq!(MEMORY_USER_ERROR_SOURCE, "memory");
    }
}
