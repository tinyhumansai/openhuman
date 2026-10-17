//! The parent agent's streamed answer text, as `text_delta` socket events.
//!
//! Kept out of the bridge loop so a held stream (`hold_text_stream`, set for
//! threads whose final reply is post-processed) is a one-line guard there.

use crate::web_chat::WebChannelEvent;

pub(super) fn publish_text_delta(
    dispatch: &crate::core::runtime::DispatchCtx,
    emit_seq: &mut u64,
    client_id: &str,
    thread_id: &str,
    request_id: &str,
    round: u32,
    delta: String,
) {
    super::publish_seq_stamped(
        dispatch,
        emit_seq,
        WebChannelEvent {
            event: "text_delta".to_string(),
            client_id: client_id.to_string(),
            thread_id: thread_id.to_string(),
            request_id: request_id.to_string(),
            round: Some(round),
            delta: Some(delta),
            delta_kind: Some("text".to_string()),
            ..Default::default()
        },
    );
}
