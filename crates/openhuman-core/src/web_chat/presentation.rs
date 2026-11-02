//! Presentation layer for web-channel chat responses: store a finished reply,
//! then announce it to the client as one unmodified `chat_done` message.

use tinychannels_bus::web as bus_web;

use crate::agent::tinyagents::host::LastTurnUsage;
use crate::web_chat::{SubagentUsagePayload, TurnUsagePayload};

use super::publish_web_channel_event_in;

fn cost_source(source: tinyagents_harness::cost::CostSource) -> bus_web::WebCostSource {
    use tinyagents_harness::cost::CostSource;
    match source {
        CostSource::Charged => bus_web::WebCostSource::Charged,
        CostSource::Estimated => bus_web::WebCostSource::Estimated,
        CostSource::Unknown => bus_web::WebCostSource::Unknown,
    }
}

/// Convert a turn's [`LastTurnUsage`] into the wire payload carried on
/// `chat_done`. Returns `None` for a turn that recorded no spend at all (e.g. a
/// synthetic budget-exhausted placeholder) so the event stays compact.
pub(super) fn usage_payload(usage: Option<&LastTurnUsage>) -> Option<TurnUsagePayload> {
    let usage = usage?;
    let subagents = usage
        .subagents
        .iter()
        .map(|s| SubagentUsagePayload {
            task_id: s.task_id.clone(),
            agent_id: s.agent_id.clone(),
            input_tokens: s.usage.input_tokens,
            output_tokens: s.usage.output_tokens,
            cost_usd: s.usage.cost().usd(),
        })
        .collect();
    Some(TurnUsagePayload {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cached_input_tokens: usage.cached_input_tokens,
        cost_usd: usage.cost_usd,
        cost_source: cost_source(usage.cost_source),
        context_window: usage.context_window,
        context_tokens: usage.context_tokens,
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
    ctx: &crate::core::runtime::DispatchCtx,
    client_id: &str,
    thread_id: &str,
    request_id: &str,
    full_response: &str,
    user_message: &str,
    citations: &[crate::web_chat::TurnCitation],
    usage: Option<&LastTurnUsage>,
    workspace_dir: Option<&std::path::Path>,
    timing: Option<bus_web::TurnTimingSnapshot>,
    suggest_follow_ups: bool,
) {
    persist_reply(
        ctx,
        thread_id,
        request_id,
        full_response,
        citations,
        workspace_dir,
    )
    .await;
    announce_reply(
        ctx,
        client_id,
        thread_id,
        request_id,
        full_response,
        user_message,
        citations,
        usage,
        timing,
        suggest_follow_ups,
    );
}

/// Stores a single-bubble reply in the thread log before it is announced
/// (#6034): once the row is on disk, a `chat_done` that is never delivered or
/// painted costs the user a repaint, not the reply. A storage failure is
/// logged and never blocks the announcement. `None` stores nothing.
pub(crate) async fn persist_reply(
    ctx: &crate::core::runtime::DispatchCtx,
    thread_id: &str,
    request_id: &str,
    full_response: &str,
    citations: &[crate::web_chat::TurnCitation],
    workspace_dir: Option<&std::path::Path>,
) {
    if let Some(dir) = workspace_dir {
        // The store appends under a process-wide lock and fsyncs, and its
        // existence check folds the whole threads log (#5156) — blocking
        // work that has no business holding a runtime worker while a turn
        // is settling. Hand it to the blocking pool and await the handle,
        // which keeps the ordering this whole change rests on.
        let (dctx, dir, thread, request, reply, cites) = (
            ctx.clone(),
            dir.to_path_buf(),
            thread_id.to_string(),
            request_id.to_string(),
            full_response.to_string(),
            citations.to_vec(),
        );
        let persisted = tokio::task::spawn_blocking(move || {
            super::reply_persistence::persist_delivered_reply(
                &dctx, &dir, &thread, &request, &reply, &cites,
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
}

/// Announces a reply already stored by [`persist_reply`]: the terminal
/// `chat_done`, then (for the main chat turn) the follow-up suggestions.
#[allow(clippy::too_many_arguments)]
pub(crate) fn announce_reply(
    ctx: &crate::core::runtime::DispatchCtx,
    client_id: &str,
    thread_id: &str,
    request_id: &str,
    full_response: &str,
    user_message: &str,
    citations: &[crate::web_chat::TurnCitation],
    usage: Option<&LastTurnUsage>,
    timing: Option<bus_web::TurnTimingSnapshot>,
    suggest_follow_ups: bool,
) {
    let usage_payload = usage_payload(usage);
    let timing_payload =
        timing.map(|snapshot| snapshot.into_payload(usage.map(|u| u.output_tokens)));
    publish_chat_done(
        ctx,
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
            ctx.clone(),
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
    ctx: &crate::core::runtime::DispatchCtx,
    client_id: &str,
    thread_id: &str,
    request_id: &str,
    full_response: &str,
    usage: Option<&LastTurnUsage>,
) {
    publish_chat_done(
        ctx,
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

/// The terminal `chat_done` for an unsegmented reply. Terminal delivery events
/// are emitted outside the seq-stamping progress stream, so `seq` stays unset
/// (older clients ignore it).
pub(super) fn chat_done_event(
    context: bus_web::WebEventContext,
    full_response: &str,
    citations: &[crate::web_chat::TurnCitation],
    usage: Option<TurnUsagePayload>,
    timing: Option<crate::web_chat::TurnTimingPayload>,
) -> bus_web::WebChannelEvent {
    bus_web::WebChannelEvent::ChatDone {
        context,
        full_response: Some(full_response.to_string()),
        citations: (!citations.is_empty()).then(|| serde_json::json!(citations)),
        usage,
        timing,
        parts: None,
    }
}

/// The terminal `chat_done` of a finished turn, ready to publish.
pub(super) fn announced_chat_done(
    context: bus_web::WebEventContext,
    full_response: &str,
    citations: &[crate::web_chat::TurnCitation],
    usage: Option<&LastTurnUsage>,
    timing: Option<bus_web::TurnTimingSnapshot>,
) -> bus_web::WebChannelEvent {
    chat_done_event(
        context,
        full_response,
        citations,
        usage_payload(usage),
        timing.map(|snapshot| snapshot.into_payload(usage.map(|u| u.output_tokens))),
    )
}

/// Emit the terminal `chat_done` for an unsegmented reply.
fn publish_chat_done(
    ctx: &crate::core::runtime::DispatchCtx,
    client_id: &str,
    thread_id: &str,
    request_id: &str,
    full_response: &str,
    citations: &[crate::web_chat::TurnCitation],
    usage_payload: Option<TurnUsagePayload>,
    timing_payload: Option<crate::web_chat::TurnTimingPayload>,
) {
    let context = bus_web::WebEventContext {
        client_id: client_id.to_string(),
        thread_id: thread_id.to_string(),
        request_id: request_id.to_string(),
        ..Default::default()
    };
    publish_web_channel_event_in(
        ctx,
        chat_done_event(
            context,
            full_response,
            citations,
            usage_payload,
            timing_payload,
        ),
    );
}

#[cfg(any(test, debug_assertions))]
#[path = "presentation_test_support_tests.rs"]
pub mod test_support;

#[cfg(test)]
#[path = "presentation_tests.rs"]
mod tests;
