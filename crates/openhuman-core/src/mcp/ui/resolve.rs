//! Turning one tool call into what the chat surface shows for it.
//!
//! The rules are `tinymcp::ui`'s. This host keeps any widget document the
//! result carried, so the frame can load it by id.

pub use tinymcp::ui::{
    https_origins, is_ui_uri, resource_from_contents, view_from_envelope, view_from_raw_result,
    MAX_STRUCTURED_BYTES, MAX_WIDGET_BYTES as MAX_RESOURCE_BYTES,
};

use super::cache::{self, InlineEntry};
use super::types::{McpUiPresentation, UiCallView};

/// The presentation for one call, or `None` when it offered neither a widget
/// nor a link. A document the result embedded is cached for the frame.
#[must_use]
pub fn presentation_from_view(view: &UiCallView) -> Option<McpUiPresentation> {
    let resolved = tinymcp::ui::resolve_presentation(view)?;
    let mut presentation = resolved.presentation;
    if let Some((uri, document)) = resolved.prefetched {
        cache::put_read(&view.server_id, &uri, document);
    }
    if let Some(document) = resolved.inline_document {
        presentation.inline_id = Some(cache::put_inline(InlineEntry {
            server_id: Some(view.server_id.clone()),
            resource: document,
        }));
    }
    Some(presentation)
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
