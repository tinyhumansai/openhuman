//! What the documents in a conversation add to its size, for compaction.
//!
//! The estimator charges every document block a flat image-sized weight, but
//! a model that cannot read a document natively is sent the document's
//! cached text readout in its place, on every request that replays it. So a
//! thread holding long documents is far larger on the wire than its
//! transcript, and a gate that trusts the estimate never compacts it. This
//! module sizes the documents the next request would actually replay by the
//! readouts cached for them.

use tinyagents_harness::multimodal::markers::{parse_file_markers, parse_image_markers};
use tinyagents_harness::token_estimation::DEFAULT_CHARS_PER_TOKEN;
use tinyinference_llm::message::{ContentBlock, MediaRef, Message};

use crate::attachments::provider::{cached_readout_chars, MAX_HISTORICAL_MEDIA_BLOCKS};
use crate::attachments::AttachmentAccessScope;
use crate::config::Config;
use crate::core::runtime::DispatchCtx;

/// The prefix of a native channel attachment's opaque reference.
const STAGED_PREFIX: &str = "openhuman-staged://";

/// The smallest fold threshold the floor allows: half of it is the most
/// verbatim tail a fold keeps, so below this the tail would shrink under 20k
/// tokens.
const FOLD_FLOOR_TOKENS: u64 = 40_000;

/// Whether `history` holds any document block at all.
pub(super) fn has_documents(history: &[Message]) -> bool {
    history.iter().any(|message| match message {
        Message::User(user) => user
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::Document(_))),
        _ => false,
    })
}

/// The document blocks the next request replays from `history`.
///
/// The provider's media wrapper replays only the newest `budget` media items
/// of the conversation's history and sends the rest as short omission
/// markers. Once a new message arrives, every message in `history` is
/// history. This walks the conversation the way that selection does: newest
/// message and newest block first, with an image, audio, video or document
/// block taking one item and a text block one per `[IMAGE:…]` or file marker
/// it holds.
pub(super) fn replayed_documents(history: &[Message], budget: usize) -> Vec<&ContentBlock> {
    let mut remaining = budget;
    let mut documents = Vec::new();
    for message in history.iter().rev() {
        let Message::User(user) = message else {
            continue;
        };
        for block in user.content.iter().rev() {
            if remaining == 0 {
                return documents;
            }
            match block {
                ContentBlock::Text(text) => {
                    let (_, files) = parse_file_markers(text);
                    let (_, images) = parse_image_markers(text);
                    remaining = remaining.saturating_sub(files.len() + images.len());
                }
                ContentBlock::Document(_) => {
                    documents.push(block);
                    remaining -= 1;
                }
                ContentBlock::Image(_) | ContentBlock::Audio(_) | ContentBlock::Video(_) => {
                    remaining -= 1;
                }
                _ => {}
            }
        }
    }
    documents
}

/// Tokens the documents the next request replays from `history` carry
/// beyond their flat estimate weight, by the readouts cached for them under
/// `scope`. A document with no cached readout (read natively, never sent,
/// or unresolvable) keeps the flat weight, so nothing is counted lower than
/// the estimate already does.
pub(super) async fn extra_tokens(
    ctx: &DispatchCtx,
    config: &Config,
    scope: &AttachmentAccessScope,
    history: &[Message],
) -> u64 {
    if config.multimodal_files.max_files == 0 {
        // Attachments are off: history media is never replayed.
        return 0;
    }
    let replayed = replayed_documents(history, MAX_HISTORICAL_MEDIA_BLOCKS);
    let mut extra_chars = 0_u64;
    let mut sized = 0_usize;
    for block in &replayed {
        let Some(path) = upload_path(ctx, config, scope, block).await else {
            continue;
        };
        let Some(chars) = cached_readout_chars(config, &path, scope).await else {
            continue;
        };
        sized += 1;
        extra_chars =
            extra_chars.saturating_add(chars.saturating_sub(block.estimated_char_weight() as u64));
    }
    let extra = (extra_chars as f64 / DEFAULT_CHARS_PER_TOKEN) as u64;
    tracing::debug!(
        replayed = replayed.len(),
        sized,
        extra_tokens = extra,
        "[compact_thread] documents sized by their cached readouts"
    );
    extra
}

/// The upload a document `block` names: its workspace path, or, for a
/// native channel attachment, the path its committed staged record
/// resolves to (as the provider resolves it before reading its readout).
async fn upload_path(
    ctx: &DispatchCtx,
    config: &Config,
    scope: &AttachmentAccessScope,
    block: &ContentBlock,
) -> Option<String> {
    let ContentBlock::Document(reference) = block else {
        return None;
    };
    let source = match reference {
        MediaRef::Path { path, .. } => path.as_str(),
        MediaRef::Url { url, .. } => url.as_str(),
        MediaRef::Base64 { .. } => return None,
    };
    let Some(opaque_id) = source.strip_prefix(STAGED_PREFIX) else {
        return matches!(reference, MediaRef::Path { .. }).then(|| source.to_owned());
    };
    let thread_id = scope.thread_id.as_deref()?;
    match crate::threads::attachments::read_authorized_native_attachment(
        ctx,
        config,
        thread_id,
        opaque_id,
        &scope.native_staged_parts,
        true,
    )
    .await
    {
        Ok(resolved) => Some(resolved.path),
        Err(error) => {
            tracing::debug!(%error, "[compact_thread] staged document not resolved");
            None
        }
    }
}

/// The threshold the fold runs against for a conversation of at least
/// `min_tokens` that the estimator measures at `measured` tokens and that
/// holds `documents` tokens of readout the estimator misses.
///
/// The fold's middleware measures documents at their flat weight, so its
/// trigger moves down by what that weight misses. It keeps
/// [`FOLD_FLOOR_TOKENS`] (or `min_tokens`, when lower) as a floor for the
/// verbatim tail, but never exceeds `measured`: a trigger above what the
/// middleware sees would find nothing to fold in a conversation the gate
/// judged large enough.
pub(super) fn fold_threshold(min_tokens: u64, documents: u64, measured: u64) -> u64 {
    min_tokens
        .saturating_sub(documents)
        .max(min_tokens.min(FOLD_FLOOR_TOKENS))
        .min(measured.max(1))
}

#[cfg(test)]
#[path = "compact_documents_tests.rs"]
mod tests;
