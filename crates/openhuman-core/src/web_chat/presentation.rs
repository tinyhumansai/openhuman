//! Presentation layer for web-channel chat responses: store a finished reply,
//! then announce it to the client as `chat_done` (or, for channel-specific
//! callers, as human-feeling `chat_segment` bubbles).

use crate::agent::tinyagents::host::LastTurnUsage;
use crate::web_chat::{SubagentUsagePayload, TurnUsagePayload, WebChannelEvent};

use super::publish_web_channel_event;
use tinychannels_bus::delivery::segment_delay;
#[cfg(test)]
use tinychannels_bus::delivery::segment_for_delivery;

/// Convert a turn's [`LastTurnUsage`] into the wire payload carried on
/// `chat_done`. Returns `None` for a turn that recorded no spend at all (e.g. a
/// synthetic budget-exhausted placeholder) so the event stays compact.
fn usage_payload(usage: Option<&LastTurnUsage>) -> Option<TurnUsagePayload> {
    let usage = usage?;
    let subagents = usage
        .subagents
        .iter()
        .map(|s| SubagentUsagePayload {
            task_id: s.task_id.clone(),
            agent_id: s.agent_id.clone(),
            input_tokens: s.usage.input_tokens,
            output_tokens: s.usage.output_tokens,
            cost_usd: s.usage.charged_amount_usd,
        })
        .collect();
    Some(TurnUsagePayload {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cached_input_tokens: usage.cached_input_tokens,
        cost_usd: usage.cost_usd,
        context_window: usage.context_window,
        subagents,
    })
}

/// Deliver one unmodified agent response to the frontend.
///
/// Desktop/web chat owns Markdown layout inside one assistant message. Splitting
/// paragraphs into `chat_segment` messages duplicates tool/reasoning parts and
/// turns one answer into several bubbles, so this path always emits exactly one
/// `chat_done` with the model's original text.
///
/// `workspace_dir` is where the reply is stored **before** it is announced, so a
/// client that never receives the `chat_done` — or receives it and fails to
/// append — is not the difference between the answer existing and not existing
/// (#6034). Pass `None` from a caller that has no workspace in scope; delivery
/// then behaves exactly as it did when the renderer was the only writer.
pub(crate) async fn deliver_response(
    client_id: &str,
    thread_id: &str,
    request_id: &str,
    full_response: &str,
    user_message: &str,
    citations: &[crate::memory::types::TurnCitation],
    usage: Option<&LastTurnUsage>,
    workspace_dir: Option<&std::path::Path>,
    timing: Option<super::turn_timing::TurnTimingSnapshot>,
    suggest_follow_ups: bool,
) {
    let usage_payload = usage_payload(usage);
    let timing_payload =
        timing.map(|snapshot| snapshot.into_payload(usage.map(|u| u.output_tokens)));

    // Keep the response byte-for-byte in one assistant message. The legacy
    // segmentation helpers remain available to channel-specific callers/tests,
    // but the interactive web surface must not cut or reformat model output.
    let segments = [full_response.to_string()];

    if segments.len() <= 1 {
        // Store the answer before announcing it. Ordering is the whole point:
        // once the row is on disk, a `chat_done` that is never delivered, never
        // painted, or never persisted by the client costs the user a repaint,
        // not the reply (#6034). Only this single-bubble branch persists — the
        // segmented branch below hands the client one row per segment to write,
        // and a full-text row beside those would read as a duplicate answer.
        if let Some(dir) = workspace_dir {
            // The store appends under a process-wide lock and fsyncs, and its
            // existence check folds the whole threads log (#5156) — blocking
            // work that has no business holding a runtime worker while a turn
            // is settling. Hand it to the blocking pool and await the handle,
            // which keeps the ordering this whole change rests on.
            let (dir, thread, request, reply, cites) = (
                dir.to_path_buf(),
                thread_id.to_string(),
                request_id.to_string(),
                full_response.to_string(),
                citations.to_vec(),
            );
            let persisted = tokio::task::spawn_blocking(move || {
                super::reply_persistence::persist_delivered_reply(
                    &dir, &thread, &request, &reply, &cites,
                )
            })
            .await;
            match persisted {
                Ok(Ok(stored)) => {
                    if stored {
                        log::debug!(
                            "[web-channel] persisted reply before announcing it thread_id={thread_id} request_id={request_id}"
                        );
                    }
                }
                // Deliberately non-fatal: announce anyway. The client's own
                // append still persists the reply in the common case, and a
                // storage failure must not also cost the user the delivery.
                Ok(Err(err)) => log::warn!(
                    "[web-channel] could not persist reply before announcing it \
                     thread_id={thread_id} request_id={request_id} error={err}"
                ),
                Err(err) => log::warn!(
                    "[web-channel] reply persistence task did not run \
                     thread_id={thread_id} request_id={request_id} error={err}"
                ),
            }
        }

        // Single bubble — emit chat_done directly.
        publish_chat_done(
            client_id,
            thread_id,
            request_id,
            full_response,
            citations,
            usage_payload,
            timing_payload,
        );
        if suggest_follow_ups {
            super::suggestions::spawn_follow_up_suggestions(
                client_id.to_string(),
                thread_id.to_string(),
                request_id.to_string(),
                user_message.to_string(),
                full_response.to_string(),
            );
        }
        return;
    }

    let total = segments.len() as u32;

    // Emit each segment as a separate bubble with a human-feeling delay.
    for (i, segment) in segments.iter().enumerate() {
        if i > 0 {
            let delay_ms = segment_delay(&segments[i - 1]);
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }

        publish_web_channel_event(WebChannelEvent {
            event: "chat_segment".to_string(),
            client_id: client_id.to_string(),
            thread_id: thread_id.to_string(),
            request_id: request_id.to_string(),
            full_response: Some(segment.clone()),
            message: None,
            error_type: None,
            error_source: None,
            error_retryable: None,
            error_retry_after_ms: None,
            error_provider: None,
            error_fallback_available: None,
            copy_key: None,
            copy_params: None,
            tool_name: None,
            skill_id: None,
            args: None,
            output: None,
            success: None,
            round: None,
            segment_index: Some(i as u32),
            segment_total: Some(total),
            delta: None,
            delta_kind: None,
            tool_call_id: None,
            failure: None,
            subagent: None,
            tool_display_label: None,
            tool_display_detail: None,
            elapsed_ms: None,
            structured: None,
            citations: if i == 0 && !citations.is_empty() {
                Some(serde_json::json!(citations))
            } else {
                None
            },
            // Usage is attached only to the terminal `chat_done`, never segments.
            usage: None,
            seq: None,
            ..Default::default()
        });
    }

    // Final chat_done with full text (for deduplication / state sync).
    publish_web_channel_event(WebChannelEvent {
        event: "chat_done".to_string(),
        client_id: client_id.to_string(),
        thread_id: thread_id.to_string(),
        request_id: request_id.to_string(),
        full_response: Some(full_response.to_string()),
        message: None,
        error_type: None,
        error_source: None,
        error_retryable: None,
        error_retry_after_ms: None,
        error_provider: None,
        error_fallback_available: None,
        copy_key: None,
        copy_params: None,
        tool_name: None,
        skill_id: None,
        args: None,
        output: None,
        success: None,
        round: None,
        segment_index: None,
        segment_total: Some(total),
        delta: None,
        delta_kind: None,
        tool_call_id: None,
        failure: None,
        subagent: None,
        tool_display_label: None,
        tool_display_detail: None,
        elapsed_ms: None,
        structured: None,
        citations: if citations.is_empty() {
            None
        } else {
            Some(serde_json::json!(citations))
        },
        usage: usage_payload,
        timing: timing_payload,
        // Terminal delivery events are emitted outside the seq-stamping
        // progress bridge; leave `seq` unset (older clients ignore it).
        seq: None,
        ..Default::default()
    });

    if suggest_follow_ups {
        super::suggestions::spawn_follow_up_suggestions(
            client_id.to_string(),
            thread_id.to_string(),
            request_id.to_string(),
            user_message.to_string(),
            full_response.to_string(),
        );
    }
}

/// Deliver an agent response as exactly one `chat_done` bubble — no
/// segmentation — for turns the core runs on its own behalf
/// (background sub-agent result delivery).
///
/// Those turns persist their closing message themselves, as a single row,
/// before announcing it (`background_delivery`). Splitting the reply
/// into `chat_segment` bubbles would have a viewing client persist one row per
/// segment beside that single row (#5933), so the conversational segmentation
/// of [`deliver_response`] is deliberately not offered here.
pub(crate) fn deliver_response_single_bubble(
    client_id: &str,
    thread_id: &str,
    request_id: &str,
    full_response: &str,
    usage: Option<&LastTurnUsage>,
) {
    publish_chat_done(
        client_id,
        thread_id,
        request_id,
        full_response,
        &[],
        usage_payload(usage),
        // Background/core-initiated turns don't run through the web-channel
        // progress bridge, so there is no `TurnTiming` to report here.
        None,
    );
}

/// Emit the terminal `chat_done` for an unsegmented reply.
fn publish_chat_done(
    client_id: &str,
    thread_id: &str,
    request_id: &str,
    full_response: &str,
    citations: &[crate::memory::types::TurnCitation],
    usage_payload: Option<TurnUsagePayload>,
    timing_payload: Option<crate::web_chat::TurnTimingPayload>,
) {
    publish_web_channel_event(WebChannelEvent {
        event: "chat_done".to_string(),
        client_id: client_id.to_string(),
        thread_id: thread_id.to_string(),
        request_id: request_id.to_string(),
        full_response: Some(full_response.to_string()),
        message: None,
        error_type: None,
        error_source: None,
        error_retryable: None,
        error_retry_after_ms: None,
        error_provider: None,
        error_fallback_available: None,
        copy_key: None,
        copy_params: None,
        tool_name: None,
        skill_id: None,
        args: None,
        output: None,
        success: None,
        round: None,
        segment_index: None,
        segment_total: None,
        delta: None,
        delta_kind: None,
        tool_call_id: None,
        failure: None,
        subagent: None,
        tool_display_label: None,
        tool_display_detail: None,
        elapsed_ms: None,
        structured: None,
        citations: if citations.is_empty() {
            None
        } else {
            Some(serde_json::json!(citations))
        },
        usage: usage_payload,
        timing: timing_payload,
        // Terminal delivery events are emitted outside the seq-stamping
        // progress bridge; leave `seq` unset (older clients ignore it).
        seq: None,
        ..Default::default()
    });
}

#[cfg(any(test, debug_assertions))]
#[path = "presentation_test_support_tests.rs"]
pub mod test_support;

#[cfg(test)]
#[path = "presentation_tests.rs"]
mod tests;
