//! Admitted channel messages as [`crate::inbound_debounce`] items, and the
//! merge that turns one debounced batch into the single message its turn runs
//! on.

use super::processor::AdmittedInbound;
use crate::inbound_debounce::DebounceItem;
use tinychannels_bus::channel::ContentPart;

const MEDIA_MARKERS: [&str; 5] = ["[IMAGE:", "[FILE:", "[VIDEO:", "[DOCUMENT:", "[Attachment:"];

/// An admitted message with its receive order on the dispatch queue.
pub(crate) struct QueuedInbound {
    pub(crate) seq: u64,
    pub(crate) inbound: AdmittedInbound,
}

fn part_is_media(part: &ContentPart) -> bool {
    !matches!(part, ContentPart::Text { .. })
}

impl DebounceItem for QueuedInbound {
    fn chars(&self) -> usize {
        self.inbound.msg.content.chars().count()
    }

    fn carries_media(&self) -> bool {
        let content = &self.inbound.msg.content;
        MEDIA_MARKERS.iter().any(|marker| content.contains(marker))
            || self.inbound.envelope.parts.iter().any(part_is_media)
    }

    fn mergeable(&self) -> bool {
        let inbound = &self.inbound;
        let trimmed = inbound.msg.content.trim_start();
        // Imported history and hosted-relay callback targets belong to exactly
        // one message, so such a message always runs as its own batch. A
        // captionless image or voice note still merges with the text around it.
        has_substance(inbound)
            && !trimmed.starts_with('/')
            && inbound.imported_history.is_empty()
            && inbound.legacy_history.is_none()
            && inbound.native_target.is_none()
    }
}

/// Whether a message contributes anything to a merged turn: some non-blank
/// text or a non-text part (an image or voice note with no caption still counts).
fn has_substance(inbound: &AdmittedInbound) -> bool {
    !inbound.msg.content.trim().is_empty()
        || inbound.envelope.parts.iter().any(|part| match part {
            ContentPart::Text { text, .. } => !text.trim().is_empty(),
            _ => true,
        })
}

/// Merge a batch into one message: ordered by (timestamp, receive order),
/// addressed like the newest message, with the non-blank bodies joined by
/// newlines and the typed content parts concatenated in the same order.
pub(crate) fn merge_channel_batch(mut batch: Vec<QueuedInbound>) -> Option<AdmittedInbound> {
    batch.sort_by_key(|item| (item.inbound.msg.timestamp, item.seq));
    if batch.len() <= 1 {
        return batch.pop().map(|item| item.inbound);
    }
    let sender_name = batch
        .iter()
        .rev()
        .find_map(|item| item.inbound.msg.sender_name.clone());
    let content = batch
        .iter()
        .map(|item| item.inbound.msg.content.as_str())
        .filter(|content| !content.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let text = batch
        .iter()
        .map(|item| item.inbound.envelope.text.as_str())
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let typed = batch.iter().any(|item| item.inbound.typed_envelope_input);
    let mut parts = Vec::new();
    let mut has_content = false;
    for item in &batch {
        if !has_substance(&item.inbound) {
            continue;
        }
        if has_content {
            parts.push(ContentPart::Text {
                text: "\n".into(),
                markdown_dialect: None,
            });
        }
        parts.extend(item.inbound.envelope.parts.iter().cloned());
        has_content = true;
    }
    let count = batch.len();
    let mut merged = batch.pop()?.inbound;
    merged.msg.content = content;
    merged.msg.sender_name = sender_name;
    merged.envelope.text = text;
    merged.envelope.parts = parts;
    merged.typed_envelope_input = typed;
    tracing::debug!(
        channel = %merged.msg.channel,
        message_id = %merged.msg.id,
        merged = count,
        "[dispatch][debounce] merged inbound batch"
    );
    Some(merged)
}

#[cfg(test)]
#[path = "inbound_batch_tests.rs"]
mod tests;
