//! A running tool's status line, as `tool_progress` socket events.
//!
//! Carries the call id of the open `tool_call` row and a human-readable
//! `message` (for a shell call: elapsed time, output written, last output
//! line). Rendering is the client's choice; the event also proves the call is
//! alive between its `tool_call` and `tool_result`.

use crate::web_chat::WebChannelEvent;

/// Publish one status line for the running call `call_id`. `ids` is
/// `(client_id, thread_id, request_id)`.
pub(super) fn publish(
    dispatch: &crate::core::runtime::DispatchCtx,
    emit_seq: &mut u64,
    (client_id, thread_id, request_id): (&str, &str, &str),
    call_id: String,
    message: String,
) {
    log::trace!("[web_channel][bridge] tool_progress call_id={call_id} request_id={request_id}");
    super::publish_seq_stamped(
        dispatch,
        emit_seq,
        WebChannelEvent {
            event: "tool_progress".to_string(),
            client_id: client_id.to_string(),
            thread_id: thread_id.to_string(),
            request_id: request_id.to_string(),
            tool_call_id: Some(call_id),
            message: Some(message),
            ..Default::default()
        },
    );
}
