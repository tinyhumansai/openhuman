//! Presentation layer for web-channel chat responses.
//!
//! Handles two concerns that run on the **local model** (zero cloud cost):
//!
//! 1. **Message segmentation** — split an agent response into human-feeling
//!    chat bubbles, but *only* when the content is natural-language prose.
//!    Code blocks, structured data, and short messages are never split.
//!
//! 2. **Emoji reactions** — decide whether the assistant should react to the
//!    user's message with an emoji.

use crate::core::socketio::{SubagentUsagePayload, TurnUsagePayload, WebChannelEvent};
use crate::openhuman::agent::harness::turn_subagent_usage::LastTurnUsage;
use crate::openhuman::config::rpc as config_rpc;

use super::publish_web_channel_event;

const MIN_SEGMENT_CHARS: usize = 40;
const MAX_SEGMENTS: usize = 5;

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
    citations: &[crate::openhuman::memory::agent::memory_loader::MemoryCitation],
    usage: Option<&LastTurnUsage>,
    workspace_dir: Option<&std::path::Path>,
) {
    let usage_payload = usage_payload(usage);

    // Spawn reaction decision in parallel — it runs on the local model and
    // shouldn't block segmentation or delivery.
    let user_msg_owned = user_message.to_string();
    let reaction_handle = tokio::spawn(async move { try_reaction(&user_msg_owned).await });

    // Keep the response byte-for-byte in one assistant message. The legacy
    // segmentation helpers remain available to channel-specific callers/tests,
    // but the interactive web surface must not cut or reformat model output.
    let segments = [full_response.to_string()];

    // Await the reaction result (should already be done or nearly done).
    let reaction_emoji = reaction_handle.await.unwrap_or(None);

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
            reaction_emoji,
            citations,
            usage_payload,
        );
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
            tool_name: None,
            skill_id: None,
            args: None,
            output: None,
            success: None,
            round: None,
            // Attach reaction emoji only on the first segment.
            reaction_emoji: if i == 0 { reaction_emoji.clone() } else { None },
            segment_index: Some(i as u32),
            segment_total: Some(total),
            delta: None,
            delta_kind: None,
            tool_call_id: None,
            failure: None,
            subagent: None,
            task_board: None,
            tool_display_label: None,
            tool_display_detail: None,
            citations: if i == 0 && !citations.is_empty() {
                Some(serde_json::json!(citations))
            } else {
                None
            },
            // Usage is attached only to the terminal `chat_done`, never segments.
            usage: None,
            seq: None,
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
        tool_name: None,
        skill_id: None,
        args: None,
        output: None,
        success: None,
        round: None,
        reaction_emoji: None,
        segment_index: None,
        segment_total: Some(total),
        delta: None,
        delta_kind: None,
        tool_call_id: None,
        failure: None,
        subagent: None,
        task_board: None,
        tool_display_label: None,
        tool_display_detail: None,
        citations: if citations.is_empty() {
            None
        } else {
            Some(serde_json::json!(citations))
        },
        usage: usage_payload,
        // Terminal delivery events are emitted outside the seq-stamping
        // progress bridge; leave `seq` unset (older clients ignore it).
        seq: None,
    });
}

/// Deliver an agent response as exactly one `chat_done` bubble — no
/// segmentation, no reaction — for turns the core runs on its own behalf
/// (autonomous task sessions, background sub-agent result delivery).
///
/// Those turns persist their closing message themselves, as a single row,
/// before announcing it (`task_session::append_final`). Splitting the reply
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
        None,
        &[],
        usage_payload(usage),
    );
}

/// Emit the terminal `chat_done` for an unsegmented reply.
fn publish_chat_done(
    client_id: &str,
    thread_id: &str,
    request_id: &str,
    full_response: &str,
    reaction_emoji: Option<String>,
    citations: &[crate::openhuman::memory::agent::memory_loader::MemoryCitation],
    usage_payload: Option<TurnUsagePayload>,
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
        tool_name: None,
        skill_id: None,
        args: None,
        output: None,
        success: None,
        round: None,
        reaction_emoji,
        segment_index: None,
        segment_total: None,
        delta: None,
        delta_kind: None,
        tool_call_id: None,
        failure: None,
        subagent: None,
        task_board: None,
        tool_display_label: None,
        tool_display_detail: None,
        citations: if citations.is_empty() {
            None
        } else {
            Some(serde_json::json!(citations))
        },
        usage: usage_payload,
        // Terminal delivery events are emitted outside the seq-stamping
        // progress bridge; leave `seq` unset (older clients ignore it).
        seq: None,
    });
}

// ── Segmentation ─────────────────────────────────────────────────────────────

/// Decide whether and how to split a response into multiple chat bubbles.
///
/// Rules (applied in order):
/// - Short messages (< 80 chars) are never split.
/// - Messages containing code fences (```) are never split.
/// - Messages that are predominantly structured (lists, tables, headers)
///   are never split — they read better as a single block.
/// - Otherwise, split on paragraph breaks (\n\n), merging segments that
///   are too short to stand alone.
/// - Fallback: split on sentence boundaries if paragraphs don't yield
///   multiple segments.
fn segment_for_delivery(text: &str) -> Vec<String> {
    let trimmed = text.trim();

    // Don't split short messages.
    if trimmed.len() < 80 {
        return vec![trimmed.to_string()];
    }

    // Never split messages containing code fences.
    if trimmed.contains("```") {
        tracing::debug!("[presentation:segment] skipping segmentation: contains code fences");
        return vec![trimmed.to_string()];
    }

    // Never split messages that are predominantly structured content.
    if is_structured_content(trimmed) {
        tracing::debug!("[presentation:segment] skipping segmentation: structured content");
        return vec![trimmed.to_string()];
    }

    // Strategy 1: paragraph splits.
    let paragraphs: Vec<&str> = trimmed
        .split("\n\n")
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();

    if paragraphs.len() >= 2 {
        let merged = merge_short(&paragraphs, "\n\n");
        if merged.len() >= 2 {
            tracing::debug!(
                segments = merged.len(),
                "[presentation:segment] split by paragraphs"
            );
            return cap_segments(merged, MAX_SEGMENTS, "\n\n");
        }
    }

    // Strategy 2: sentence splits.
    let sentences = split_sentences(trimmed);
    if sentences.len() >= 2 {
        let grouped = group_sentences(&sentences);
        if grouped.len() >= 2 {
            tracing::debug!(
                segments = grouped.len(),
                "[presentation:segment] split by sentences"
            );
            return cap_segments(grouped, MAX_SEGMENTS, " ");
        }
    }

    // Fallback: single bubble.
    vec![trimmed.to_string()]
}

/// Returns true if the text is predominantly structured content that
/// shouldn't be split across bubbles (markdown lists, tables, headers).
fn is_structured_content(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return false;
    }

    let structured_count = lines
        .iter()
        .filter(|line| {
            let trimmed = line.trim();
            trimmed.starts_with("- ")
                || trimmed.starts_with("* ")
                || trimmed.starts_with("| ")
                || trimmed.starts_with("# ")
                || trimmed.starts_with("## ")
                || trimmed.starts_with("### ")
                || is_numbered_list_item(trimmed)
        })
        .count();

    // If more than 40% of non-empty lines are structured, don't split.
    let non_empty = lines.iter().filter(|l| !l.trim().is_empty()).count();
    non_empty > 0 && (structured_count * 100 / non_empty) > 40
}

/// Check if a line starts with a numbered list prefix like "1. " or "12. ".
/// Rejects dates ("2024. ") and decimals by requiring the digits+dot+space
/// to appear at the very start and be followed by text.
fn is_numbered_list_item(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut i = 0;
    // Consume one or more leading ASCII digits.
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    // Must have consumed at least one digit, followed by ". ".
    i > 0 && i <= 3 && bytes.get(i) == Some(&b'.') && bytes.get(i + 1) == Some(&b' ')
}

/// Cap the number of delivered segments at `max` without losing content:
/// the first `max - 1` segments are kept as-is, and any overflow is
/// concatenated into a single trailing segment using `joiner`.
///
/// The earlier behavior (`.take(MAX_SEGMENTS)`) silently dropped every
/// segment past the cap, which truncated long agent replies in the UI
/// (issue #1041). Merging into the tail preserves all content while
/// still bounding the inter-bubble delay budget.
fn cap_segments(segments: Vec<String>, max: usize, joiner: &str) -> Vec<String> {
    if max == 0 || segments.len() <= max {
        return segments;
    }
    let original_len = segments.len();
    let mut iter = segments.into_iter();
    let mut result: Vec<String> = (&mut iter).take(max - 1).collect();
    let tail: Vec<String> = iter.collect();
    let tail_count = tail.len();
    let merged = tail.join(joiner);
    tracing::debug!(
        target: "presentation",
        max,
        original_len,
        tail_count,
        tail_len = merged.len(),
        joiner_len = joiner.len(),
        "[presentation:segment] merging {} overflow segments into tail",
        tail_count
    );
    result.push(merged);
    result
}

/// Merge adjacent segments shorter than MIN_SEGMENT_CHARS.
fn merge_short(parts: &[&str], joiner: &str) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for part in parts {
        if !result.is_empty() && part.len() < MIN_SEGMENT_CHARS {
            let last = result.last_mut().unwrap();
            last.push_str(joiner);
            last.push_str(part);
        } else {
            result.push(part.to_string());
        }
    }
    result
}

/// Split text on sentence-ending punctuation (. ! ?) followed by a space
/// and an uppercase letter.
fn split_sentences(text: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();

    let mut i = 0;
    while i < chars.len() {
        current.push(chars[i]);
        let ch = chars[i];

        // Latin sentence terminators: split on ". " followed by uppercase.
        if (ch == '.' || ch == '!' || ch == '?')
            && i + 2 < chars.len()
            && chars[i + 1] == ' '
            && chars[i + 2].is_ascii_uppercase()
        {
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                parts.push(trimmed);
            }
            current.clear();
            i += 2; // skip the space
            continue;
        }

        // CJK sentence terminators: split after fullwidth period/exclamation/question.
        if (ch == '\u{3002}' || ch == '\u{FF01}' || ch == '\u{FF1F}') && i + 1 < chars.len() {
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                parts.push(trimmed);
            }
            current.clear();
            i += 1;
            continue;
        }

        i += 1;
    }

    let remaining = current.trim().to_string();
    if !remaining.is_empty() {
        parts.push(remaining);
    }
    parts
}

/// Group sentences into 2-3 bubbles.
fn group_sentences(sentences: &[String]) -> Vec<String> {
    let target_count = std::cmp::min(3, sentences.len().div_ceil(2));
    let group_size = sentences.len().div_ceil(target_count);
    let mut groups: Vec<String> = Vec::new();

    for chunk in sentences.chunks(group_size) {
        let joined = chunk.join(" ");
        if joined.len() >= MIN_SEGMENT_CHARS {
            groups.push(joined);
        } else if let Some(last) = groups.last_mut() {
            last.push(' ');
            last.push_str(&joined);
        } else {
            groups.push(joined);
        }
    }
    groups
}

/// Compute a human-feeling inter-bubble delay in milliseconds.
/// Bounded: 500ms–1400ms, scaling with segment length.
fn segment_delay(segment: &str) -> u64 {
    let base: u64 = 500;
    let per_char: u64 = 2; // ~1.5-2ms per char for a natural reading pace
    std::cmp::min(base + (segment.len() as u64) * per_char, 1400)
}

// ── Reactions ────────────────────────────────────────────────────────────────

/// Ask the local model for an emoji reaction to the user's message.
/// Returns `None` if the local model is unavailable or decides no reaction.
async fn try_reaction(user_message: &str) -> Option<String> {
    if user_message.trim().is_empty() {
        return None;
    }

    let config = match config_rpc::load_config_with_timeout().await {
        Ok(c) => c,
        Err(_) => return None,
    };

    if !config.local_ai.runtime_enabled {
        return None;
    }

    match crate::openhuman::inference::ops::inference_should_react(&config, user_message, "web")
        .await
    {
        Ok(outcome) => {
            let decision = outcome.value;
            if decision.should_react {
                decision.emoji
            } else {
                None
            }
        }
        Err(e) => {
            tracing::debug!(error = %e, "[presentation:reaction] local model reaction failed");
            None
        }
    }
}

#[cfg(any(test, debug_assertions))]
#[path = "presentation_test_support_tests.rs"]
pub mod test_support;

#[cfg(test)]
#[path = "presentation_tests.rs"]
mod tests;
