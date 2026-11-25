use anyhow::Result;
use tinyinference_llm::message::{ContentBlock, Message, UserMessage};

/// Keep host-added context while retaining already-resolved, ordered media
/// blocks on a trusted typed user input. The string enrichment path remains
/// authoritative for text-only turns.
pub(super) fn enriched_typed_user_input(
    original: &Message,
    original_user_message: &str,
    enriched: &str,
) -> Result<Option<Message>, ()> {
    let Message::User(user) = original else {
        return Ok(None);
    };
    let has_typed_media = user.content.iter().any(|block| {
        matches!(
            block,
            ContentBlock::Image(_)
                | ContentBlock::Audio(_)
                | ContentBlock::Video(_)
                | ContentBlock::Document(_)
        )
    });
    if !has_typed_media {
        return Ok(None);
    }
    // `enrich_request` only prepends host context and the turn preamble to
    // `original_user_message`. If that invariant changes, fail the turn rather
    // than downgrade trusted typed media into a string.
    let host_prefix = enriched.strip_suffix(original_user_message).ok_or(())?;
    let mut content = Vec::with_capacity(user.content.len() + 1);
    if !host_prefix.is_empty() {
        content.push(ContentBlock::Text(host_prefix.to_owned()));
    }
    content.extend(user.content.iter().cloned());
    Ok(Some(Message::User(UserMessage { content })))
}
