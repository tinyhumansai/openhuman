//! Sentry filter for expected missing channel-message relay rows.
//!
//! A 404 on PATCH/DELETE to a channel-message path is an expected backend
//! state (the user deleted the message provider-side, the backend GC'd the
//! relay row). The delivery path already suppresses it; this filter is the
//! outermost safety net for any future call site that bypasses it, installed in
//! the core's crash-noise slot by [`register`]. Targets OPENHUMAN-TAURI-R7.

use openhuman_embed::__host::core::observability::{register_crash_noise_filter, CrashNoiseFilter};
use openhuman_embed::process::sentry::Event;

/// The filter's name in the `before_send` drop log line.
pub(crate) const FILTER_NAME: &str = "channel-message-404";

/// Whether `event` is a backend 404 on a channel-message PATCH/DELETE.
///
/// Match criteria (all required):
/// - tag `domain == "backend_api"`
/// - tag `failure == "non_2xx"`
/// - tag `status == "404"`
/// - tag `method == "PATCH"` or `"DELETE"`
/// - event message or exception value contains both `"/channels/"` and `"/messages/"`
pub fn is_channel_message_not_found_event(event: &Event<'_>) -> bool {
    let tags = &event.tags;
    if tags.get("domain").map(String::as_str) != Some("backend_api") {
        return false;
    }
    if tags.get("failure").map(String::as_str) != Some("non_2xx") {
        return false;
    }
    if tags.get("status").map(String::as_str) != Some("404") {
        return false;
    }
    let method = tags.get("method").map(String::as_str).unwrap_or("");
    if method != "PATCH" && method != "DELETE" {
        return false;
    }
    event_contains_channel_message_path(event)
}

fn event_contains_channel_message_path(event: &Event<'_>) -> bool {
    let has_pattern = |s: &str| s.contains("/channels/") && s.contains("/messages/");
    if event.message.as_deref().is_some_and(has_pattern) {
        return true;
    }
    event
        .exception
        .values
        .iter()
        .any(|exc| exc.value.as_deref().is_some_and(has_pattern))
}

fn matches_static(event: &Event<'static>) -> bool {
    is_channel_message_not_found_event(event)
}

/// Installs the filter in the core's crash-noise slot (idempotent).
pub(crate) fn register() {
    log::debug!("[tinyhumans] channels: registering the {FILTER_NAME} crash filter");
    register_crash_noise_filter(CrashNoiseFilter {
        name: FILTER_NAME,
        matches: matches_static,
    });
}

#[cfg(test)]
#[path = "message_filter_tests.rs"]
mod tests;
