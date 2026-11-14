//! The browser-pictures hook: where pictures of browser pages go.
//!
//! When a host registers a sink here, the agent's `browser` tool takes a
//! picture of the page by itself after it opens one, after a click, key
//! press, select or check, and before it closes a session
//! (`tools::impl::browser::capture`), and hands each picture to every
//! registered sink. With no sink registered, no picture is taken.
//!
//! The core keeps nothing: what a picture is for (a report, a post, a log)
//! and where it is kept are the host's. A picture never shows a password the
//! keychain typed: the tool skips that page before any sink sees it.

use std::sync::Arc;

use crate::core::slots::HookSlot;

/// One picture of a browser page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserPicture {
    /// The image, as `media_type`.
    pub bytes: Vec<u8>,
    /// `image/png`, or `image/jpeg` for a page whose PNG was too large.
    pub media_type: &'static str,
    /// The browser action the picture follows (`open`, `click`, `close`, ...).
    pub after: String,
    /// When it was taken, in milliseconds since the Unix epoch.
    pub taken_at_ms: u128,
}

/// A registered sink. It runs on the tool's task, so it should be quick; an
/// error is logged and never fails the browser call.
pub type BrowserPictureSink = Arc<dyn Fn(&BrowserPicture) -> Result<(), String> + Send + Sync>;

static SINKS: HookSlot<BrowserPictureSink> = HookSlot::new("modules::browser_pictures");

/// Registers `sink` under `id` for the whole process. Re-registering an id
/// replaces it in place.
pub fn register_browser_picture_sink<F>(id: impl Into<String>, sink: F)
where
    F: Fn(&BrowserPicture) -> Result<(), String> + Send + Sync + 'static,
{
    SINKS.register(id, Arc::new(sink));
}

/// Removes the process-wide sink `id`.
pub fn unregister_browser_picture_sink(id: &str) -> bool {
    SINKS.unregister(id)
}

/// Whether any sink wants pictures.
pub(crate) fn wanted() -> bool {
    !SINKS.entries().is_empty()
}

/// Hands `picture` to every registered sink; returns how many took it.
pub(crate) fn deliver(picture: &BrowserPicture) -> usize {
    let mut took = 0;
    for (id, sink) in SINKS.entries() {
        match sink(picture) {
            Ok(()) => took += 1,
            Err(error) => {
                tracing::warn!(sink = %id, error = %error, "[browser-pictures] a sink refused a picture")
            }
        }
    }
    took
}

#[cfg(test)]
#[path = "browser_pictures_tests.rs"]
mod tests;
