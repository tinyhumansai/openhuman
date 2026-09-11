//! Debug-build seams for raw integration coverage of channel inbound helpers.

use super::*;

pub fn extract_message_id_for_test(resp: &serde_json::Value) -> Option<String> {
    extract_message_id(resp)
}

pub fn compose_draft_for_test(content: &str) -> String {
    let state = StreamingState {
        content: content.to_string(),
        ..Default::default()
    };
    state.compose_draft()
}

pub fn latest_thinking_snippet_for_test(thinking: &str) -> Option<String> {
    let state = StreamingState {
        thinking_accumulator: thinking.to_string(),
        ..Default::default()
    };
    latest_thinking_snippet(&state)
}

pub fn derive_inbound_thread_id_for_test(
    channel: &str,
    sender: Option<&str>,
    reply_target: Option<&str>,
    thread_ts: Option<&str>,
) -> String {
    derive_inbound_thread_id(channel, sender, reply_target, thread_ts)
}
