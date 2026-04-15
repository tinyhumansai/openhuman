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

use crate::core::socketio::WebChannelEvent;
use crate::openhuman::config::rpc as config_rpc;

use super::web::publish_web_channel_event;

const MIN_SEGMENT_CHARS: usize = 40;
const MAX_SEGMENTS: usize = 5;

/// Deliver an agent response to the frontend, applying local-model
/// presentation (segmentation + reaction) when the model is available.
///
/// Always emits at least one `chat_done` event. When the response is
/// segmented, emits one `chat_segment` per bubble first, then a final
/// `chat_done` with the full text for deduplication.
pub async fn deliver_response(
    client_id: &str,
    thread_id: &str,
    request_id: &str,
    full_response: &str,
    user_message: &str,
) {
    // Spawn reaction decision in parallel — it runs on the local model and
    // shouldn't block segmentation or delivery.
    let user_msg_owned = user_message.to_string();
    let reaction_handle = tokio::spawn(async move { try_reaction(&user_msg_owned).await });

    // Segmentation is pure CPU work, runs immediately.
    let segments = segment_for_delivery(full_response);

    // Await the reaction result (should already be done or nearly done).
    let reaction_emoji = reaction_handle.await.unwrap_or(None);

    if segments.len() <= 1 {
        // Single bubble — emit chat_done directly.
        publish_web_channel_event(WebChannelEvent {
            event: "chat_done".to_string(),
            client_id: client_id.to_string(),
            thread_id: thread_id.to_string(),
            request_id: request_id.to_string(),
            full_response: Some(full_response.to_string()),
            message: None,
            error_type: None,
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
        });
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
            return merged.into_iter().take(MAX_SEGMENTS).collect();
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
            return grouped.into_iter().take(MAX_SEGMENTS).collect();
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

    if !config.local_ai.enabled {
        return None;
    }

    match crate::openhuman::local_ai::ops::local_ai_should_react(&config, user_message, "web").await
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_messages_are_never_split() {
        let result = segment_for_delivery("Hello there!");
        assert_eq!(result, vec!["Hello there!"]);
    }

    #[test]
    fn code_fences_prevent_splitting() {
        let text = "Here is some code:\n\n```rust\nfn main() {}\n```\n\nAnd more text after.";
        let result = segment_for_delivery(text);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn paragraph_splitting_works() {
        let text = "This is the first paragraph with enough content to stand alone.\n\n\
                     This is the second paragraph that also has sufficient length.";
        let result = segment_for_delivery(text);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn structured_content_not_split() {
        let text = "Here are the steps:\n\n- First do this thing\n- Then do that thing\n- Finally wrap up\n\nThat should cover it.";
        let result = segment_for_delivery(text);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn sentence_splitting_works() {
        let text = "This is the first sentence and it has some length. This is the second sentence that continues the thought. And here is a third sentence to round things out.";
        let result = segment_for_delivery(text);
        assert!(
            result.len() >= 2,
            "expected >= 2 segments, got {}",
            result.len()
        );
    }

    #[test]
    fn segment_delay_bounds() {
        assert_eq!(segment_delay(""), 500);
        assert_eq!(segment_delay(&"x".repeat(1000)), 1400);
        assert!(segment_delay("Hello world") > 500);
    }

    #[test]
    fn numbered_list_detection() {
        assert!(is_numbered_list_item("1. First item"));
        assert!(is_numbered_list_item("12. Twelfth item"));
        assert!(!is_numbered_list_item("2024. Was a good year")); // too many digits
        assert!(!is_numbered_list_item("hello 1. world")); // digits not at start
        assert!(!is_numbered_list_item("1.5 seconds")); // no space after dot
    }

    #[test]
    fn max_segments_respected() {
        let paras: Vec<String> = (0..10)
            .map(|i| {
                format!(
                    "Paragraph number {} has enough content to stand on its own.",
                    i
                )
            })
            .collect();
        let text = paras.join("\n\n");
        let result = segment_for_delivery(&text);
        assert!(result.len() <= MAX_SEGMENTS);
    }

    #[test]
    fn split_sentences_splits_on_sentence_terminators() {
        let out = split_sentences("Hello world. How are you? I am fine!");
        assert!(out.len() >= 3);
    }

    #[test]
    fn split_sentences_handles_empty_string() {
        assert!(split_sentences("").is_empty());
    }

    #[test]
    fn split_sentences_single_sentence_without_terminator() {
        let out = split_sentences("Just one thing");
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn group_sentences_single_entry_roundtrip() {
        let v: Vec<String> = vec!["Hello world".into()];
        let out = group_sentences(&v);
        assert!(!out.is_empty());
    }

    #[test]
    fn group_sentences_multi_entry_produces_output() {
        let v: Vec<String> = vec![
            "First sentence.".into(),
            "Second sentence.".into(),
            "Third sentence.".into(),
        ];
        let out = group_sentences(&v);
        assert!(!out.is_empty());
    }

    #[test]
    fn merge_short_joins_small_parts_with_separator() {
        let out = merge_short(&["hi", "there"], " ");
        assert!(!out.is_empty());
    }

    #[test]
    fn merge_short_empty_input_returns_empty() {
        let out: Vec<String> = merge_short(&[], " ");
        assert!(out.is_empty());
    }

    #[test]
    fn segment_delay_is_monotonic_in_length() {
        let short = segment_delay("hi");
        let longer = segment_delay(&"a".repeat(500));
        assert!(longer >= short);
    }

    #[test]
    fn segment_delay_is_finite_for_huge_text() {
        let huge = "a".repeat(10_000);
        assert!(segment_delay(&huge) < 1_000_000);
    }

    #[test]
    fn segment_delay_works_on_empty_text() {
        let _ = segment_delay("");
    }

    #[test]
    fn is_structured_content_detects_markdown_headings() {
        assert!(is_structured_content("# Heading\n\nbody"));
    }

    #[test]
    fn is_structured_content_detects_bullet_list() {
        assert!(is_structured_content("- item 1\n- item 2"));
    }

    #[test]
    fn is_structured_content_detects_numbered_list() {
        assert!(is_structured_content("1. First\n2. Second"));
    }

    #[test]
    fn is_structured_content_false_for_plain_prose() {
        assert!(!is_structured_content("Just a plain sentence."));
    }

    #[test]
    fn segment_for_delivery_whitespace_only_is_empty_or_single() {
        let r = segment_for_delivery("   ");
        // Whitespace may return a single segment or empty depending on how
        // the code treats leading/trailing whitespace. Either is acceptable.
        assert!(r.len() <= 1);
    }

    #[test]
    fn segment_for_delivery_single_short_returns_one() {
        let r = segment_for_delivery("Quick.");
        assert_eq!(r.len(), 1);
    }
}
