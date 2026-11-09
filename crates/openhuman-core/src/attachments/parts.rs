//! Typed attachments on a chat request.
//!
//! The web chat RPC (`channel.web_chat`), `threads.message_append` and
//! `threads.edit_message` take attachments as typed
//! [`tinychannels_bus::ContentPart`]s next to the message text, instead of
//! `[IMAGE:…]` / `[FILE:…]` markers a client has to splice into it. The marker
//! grammar stays an internal staging format: [`fold_into_message`] renders the
//! parts into it once, at the RPC edge, and everything downstream (staging,
//! limits, conversion, the durable `[ATTACHMENT:…]` references) is unchanged.
//!
//! Only parts that carry their bytes inline (`data:` URI) or by remote URL are
//! accepted. A `path` or `localCachePath` is refused so the typed field cannot
//! be used to name a local file the way a forged marker could.

use tinychannels_bus::{ContentPart, MediaReference};

const MAX_PARTS: usize = 32;

/// Why a typed attachment could not be accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachmentPartError {
    TooMany { count: usize },
    UnsupportedKind { index: usize, kind: &'static str },
    NoSource { index: usize },
    LocalPath { index: usize },
    BadUrl { index: usize },
}

impl std::fmt::Display for AttachmentPartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooMany { count } => {
                write!(f, "too many attachments ({count}, max {MAX_PARTS})")
            }
            Self::UnsupportedKind { index, kind } => write!(
                f,
                "attachment {index}: {kind} parts are not supported as chat attachments"
            ),
            Self::NoSource { index } => write!(
                f,
                "attachment {index}: media needs an inline data: URI or an http(s) url"
            ),
            Self::LocalPath { index } => {
                write!(f, "attachment {index}: local paths are not accepted")
            }
            Self::BadUrl { index } => write!(
                f,
                "attachment {index}: url must not contain whitespace or ']'"
            ),
        }
    }
}

impl std::error::Error for AttachmentPartError {}

fn kind_name(part: &ContentPart) -> &'static str {
    match part {
        ContentPart::Text { .. } => "text",
        ContentPart::Image { .. } => "image",
        ContentPart::Video { .. } => "video",
        ContentPart::Audio { .. } => "audio",
        ContentPart::File { .. } => "file",
        ContentPart::VoiceNote { .. } => "voice_note",
        ContentPart::Sticker { .. } => "sticker",
        ContentPart::Location { .. } => "location",
        ContentPart::Contact { .. } => "contact",
        ContentPart::Buttons { .. } => "buttons",
        ContentPart::Poll { .. } => "poll",
        ContentPart::Presentation { .. } => "presentation",
        ContentPart::Native { .. } => "native",
    }
}

fn source_of(media: &MediaReference, index: usize) -> Result<&str, AttachmentPartError> {
    if media.path.is_some() || media.local_cache_path.is_some() {
        return Err(AttachmentPartError::LocalPath { index });
    }
    let url = media
        .url
        .as_deref()
        .ok_or(AttachmentPartError::NoSource { index })?;
    let remote = url.starts_with("https://") || url.starts_with("http://");
    if !url.starts_with("data:") && !remote {
        return Err(AttachmentPartError::NoSource { index });
    }
    if url.contains(']') || url.chars().any(char::is_whitespace) {
        return Err(AttachmentPartError::BadUrl { index });
    }
    Ok(url)
}

/// Render `parts` after `text` as the staging markers, in order.
///
/// `Text` parts join the message text. Images become `[IMAGE:…]`; video, audio
/// and files become `[FILE:…]`. Voice notes retain their typed audio payload
/// at this boundary and contribute no marker to the model text.
pub fn fold_into_message(text: &str, parts: &[ContentPart]) -> Result<String, AttachmentPartError> {
    if parts.is_empty() {
        return Ok(text.to_string());
    }
    if parts.len() > MAX_PARTS {
        return Err(AttachmentPartError::TooMany { count: parts.len() });
    }
    let mut words: Vec<String> = Vec::new();
    let mut markers: Vec<String> = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        match part {
            ContentPart::Text { text, .. } => words.push(text.trim().to_string()),
            ContentPart::Image { media, .. } => {
                markers.push(format!("[IMAGE:{}]", source_of(media, index)?));
            }
            ContentPart::Video { media, .. }
            | ContentPart::Audio { media, .. }
            | ContentPart::File { media, .. } => {
                markers.push(format!("[FILE:{}]", source_of(media, index)?));
            }
            ContentPart::VoiceNote { voice_note } => {
                // The handler validates this opaque reference against the
                // durable note store. Never expose it in model-visible text.
                let _ = voice_note;
            }
            other => {
                return Err(AttachmentPartError::UnsupportedKind {
                    index,
                    kind: kind_name(other),
                });
            }
        }
    }
    let mut out = text.trim().to_string();
    for chunk in words
        .into_iter()
        .filter(|chunk| !chunk.is_empty())
        .chain(markers)
    {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&chunk);
    }
    log::debug!(
        "[attachments] folded typed attachments into message parts={}",
        parts.len()
    );
    Ok(out)
}

#[cfg(test)]
#[path = "parts_tests.rs"]
mod tests;
