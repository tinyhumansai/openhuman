//! Provider-surface handoff — routes companion-identified connected-app
//! actions to the provider_surfaces queue.
//!
//! When the companion LLM response mentions a specific connected app
//! (e.g. "reply to the Slack message"), this module checks whether a
//! matching [`RespondQueueItem`] exists in the provider_surfaces queue
//! and emits a [`HandoffEvent`] through the companion event bus.
//!
//! This is intentionally light-touch: the provider_surfaces domain is
//! scaffolding-complete but behaviorally incomplete — we wire the
//! plumbing so it works when the surface is ready.

use log::debug;
use serde::{Deserialize, Serialize};

use crate::openhuman::provider_surfaces::store;
use crate::openhuman::provider_surfaces::types::RespondQueueItem;

const LOG_PREFIX: &str = "[companion_handoff]";

/// Known provider keywords the LLM might reference.
const PROVIDER_KEYWORDS: &[(&str, &str)] = &[
    ("slack", "slack"),
    ("discord", "discord"),
    ("telegram", "telegram"),
    ("whatsapp", "whatsapp"),
    ("imessage", "imessage"),
    ("email", "gmail"),
    ("gmail", "gmail"),
    ("google meet", "google-meet"),
];

/// A handoff event emitted when the companion identifies a provider action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffEvent {
    /// The provider name that was matched (e.g. "slack").
    pub provider: String,
    /// Queue items from the provider_surfaces queue that match.
    pub matching_items: Vec<RespondQueueItem>,
    /// The original LLM response text that triggered the handoff.
    pub response_text: String,
}

/// Check the LLM response for provider references and match against the
/// provider_surfaces queue. Returns a list of handoff events (usually 0 or 1).
pub fn check_handoff(response_text: &str) -> Vec<HandoffEvent> {
    if response_text.is_empty() {
        return Vec::new();
    }

    let queue_items = store::list_queue_items();

    if queue_items.is_empty() {
        debug!("{LOG_PREFIX} no items in provider queue, skipping handoff check");
        return Vec::new();
    }

    check_handoff_with_items(response_text, &queue_items)
}

/// Pure matching logic: match provider keywords against response text and the
/// given queue items. Extracted so tests can exercise the positive path without
/// depending on global store state.
pub(crate) fn check_handoff_with_items(
    response_text: &str,
    queue_items: &[RespondQueueItem],
) -> Vec<HandoffEvent> {
    let lower = response_text.to_lowercase();
    let mut events = Vec::new();

    // Split response into tokens once for word-boundary matching.
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '-')
        .filter(|s| !s.is_empty())
        .collect();

    for &(keyword, provider_id) in PROVIDER_KEYWORDS {
        // Token-aware match: single-word keywords use exact token match to avoid
        // substring false positives (e.g. "slacking" won't match "slack").
        // Multi-word keywords (like "google meet") fall back to substring match.
        let matched = if keyword.contains(' ') {
            lower.contains(keyword)
        } else {
            tokens.iter().any(|t| *t == keyword)
        };
        if !matched {
            continue;
        }

        // Deduplicate: skip if we already emitted an event for this provider
        // (e.g. "email" and "gmail" both map to provider_id "gmail").
        if events
            .iter()
            .any(|e: &HandoffEvent| e.provider == provider_id)
        {
            debug!("{LOG_PREFIX} skipping duplicate provider={provider_id} (already matched)");
            continue;
        }

        let matching: Vec<RespondQueueItem> = queue_items
            .iter()
            .filter(|item| item.provider.to_lowercase() == provider_id)
            .cloned()
            .collect();

        if matching.is_empty() {
            debug!("{LOG_PREFIX} keyword '{keyword}' found but no queue items for provider={provider_id}");
            continue;
        }

        debug!(
            "{LOG_PREFIX} handoff: keyword='{keyword}' provider={provider_id} matches={}",
            matching.len()
        );

        events.push(HandoffEvent {
            provider: provider_id.to_string(),
            matching_items: matching,
            response_text: response_text.to_string(),
        });
    }

    events
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_queue_item(provider: &str) -> RespondQueueItem {
        RespondQueueItem {
            id: "test-id".into(),
            provider: provider.into(),
            account_id: "acct".into(),
            event_kind: "message".into(),
            entity_id: "ent".into(),
            thread_id: None,
            title: None,
            snippet: None,
            sender_name: None,
            sender_handle: None,
            timestamp: "2026-01-01T00:00:00Z".into(),
            deep_link: None,
            requires_attention: true,
            status: String::new(),
        }
    }

    #[test]
    fn check_handoff_empty_response() {
        assert!(check_handoff("").is_empty());
    }

    #[test]
    fn check_handoff_no_keywords() {
        let events = check_handoff("Please click the save button.");
        assert!(events.is_empty());
    }

    #[test]
    fn check_handoff_keyword_but_empty_queue() {
        // Queue is empty by default in tests.
        let events = check_handoff("Reply to the Slack message from Alice.");
        assert!(events.is_empty());
    }

    #[test]
    fn check_handoff_with_items_emits_event() {
        let items = vec![make_queue_item("slack")];
        let events = check_handoff_with_items("Reply to the Slack message", &items);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].provider, "slack");
        assert_eq!(events[0].matching_items.len(), 1);
    }

    #[test]
    fn check_handoff_with_items_deduplicates_gmail() {
        // "email" and "gmail" both map to provider_id "gmail" — should emit once.
        let items = vec![make_queue_item("gmail")];
        let events = check_handoff_with_items("Forward the email from Gmail to the team", &items);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].provider, "gmail");
    }

    #[test]
    fn check_handoff_with_items_no_substring_false_positive() {
        // "slacking" should NOT match "slack".
        let items = vec![make_queue_item("slack")];
        let events = check_handoff_with_items("Stop slacking off", &items);
        assert!(events.is_empty());
    }

    #[test]
    fn check_handoff_with_items_multi_word_keyword() {
        let items = vec![make_queue_item("google-meet")];
        let events = check_handoff_with_items("Join the Google Meet call", &items);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].provider, "google-meet");
    }

    #[test]
    fn provider_keywords_cover_known_providers() {
        let providers: Vec<&str> = PROVIDER_KEYWORDS.iter().map(|(_, p)| *p).collect();
        assert!(providers.contains(&"slack"));
        assert!(providers.contains(&"discord"));
        assert!(providers.contains(&"telegram"));
        assert!(providers.contains(&"whatsapp"));
        assert!(providers.contains(&"gmail"));
    }

    #[test]
    fn provider_keywords_case_insensitive_match() {
        // Route through production matcher to verify case handling.
        let items = vec![make_queue_item("slack")];
        let events = check_handoff_with_items("Check your SLACK messages", &items);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].provider, "slack");
    }
}
