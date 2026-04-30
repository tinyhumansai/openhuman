//! Slack user-id → display-name resolver.
//!
//! Slack's `conversations.history` payload references users by their
//! workspace-stable id (e.g. `U01Q1TBL20P`) in two places:
//!
//!   1. The `user` field on each message (the author).
//!   2. Inline `<@U01Q1TBL20P>` mention syntax inside `text`.
//!
//! Neither is human-readable. To make canonical chat transcripts useful
//! for retrieval (and for humans reading the seal-cascade summaries),
//! we fetch the workspace's user directory once per sync run, build an
//! id → display-name map, and apply it both to the author field and to
//! every `<@…>` mention in message bodies.
//!
//! ## Cache scope
//!
//! Per-sync only. Each `SlackProvider::sync()` invocation calls
//! [`SlackUsers::fetch`] once before walking channels. The map lives
//! in a local variable for the duration of the sync, then drops.
//! Slack's user list rarely changes within a 15-minute sync window,
//! and re-fetching per sync keeps stale-cache risk near zero without
//! adding persistence machinery.
//!
//! ## Soft-fallback contract
//!
//! Following the pattern of [`crate::openhuman::composio::providers::slack::sync::extract_messages`]
//! and the [`super::provider::SlackProvider::sync`] error handling, a
//! failure to fetch users is **not fatal**. The returned [`SlackUsers`]
//! is empty, and `resolve()` / `replace_mentions()` pass through raw
//! ids unchanged — same behaviour as before this module existed.

use regex::Regex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::openhuman::composio::client::ComposioClient;

/// Composio action slug for the bulk user listing.
const ACTION_LIST_USERS: &str = "SLACK_LIST_ALL_USERS";

/// Page size — Slack caps at 1000; 200 keeps each page small.
const PAGE_SIZE: u32 = 200;

/// Maximum pages to walk per sync. With `PAGE_SIZE = 200` this covers
/// workspaces up to 4000 users without complaint. Beyond that the tail
/// is truncated and unresolved ids will pass through verbatim.
const MAX_PAGES: u32 = 20;

/// Slack mention syntax: `<@U01Q1TBL20P>`. Captures the bare id so we
/// can drop the wrapper when substituting in a resolved name.
fn mention_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<@(U[A-Z0-9]+)>").expect("static mention regex compiles"))
}

/// Map of Slack user id → human-readable display name.
#[derive(Debug, Default, Clone)]
pub struct SlackUsers {
    map: HashMap<String, String>,
}

impl SlackUsers {
    /// Empty map — `resolve()` passes through raw ids verbatim.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Number of users in the cache.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Resolve a Slack user id to a display name. Returns the input id
    /// unchanged when no mapping exists — matches the
    /// resolve-or-passthrough contract of the parent provider.
    pub fn resolve(&self, user_id: &str) -> String {
        self.map
            .get(user_id)
            .cloned()
            .unwrap_or_else(|| user_id.to_string())
    }

    /// Replace every `<@Uxxx>` mention in `text` with `@<display name>`.
    /// Unknown ids stay as `@Uxxx` (the wrapper is removed but the id
    /// is preserved so retrieval can still surface them).
    pub fn replace_mentions(&self, text: &str) -> String {
        mention_re()
            .replace_all(text, |caps: &regex::Captures| {
                let id = &caps[1];
                let resolved = self.map.get(id).map(String::as_str).unwrap_or(id);
                format!("@{resolved}")
            })
            .into_owned()
    }

    /// Pull the workspace user directory via Composio. Soft-fails to
    /// [`SlackUsers::empty`] on transport, HTTP, JSON, or
    /// provider-failure errors so the sync can continue with raw ids.
    ///
    /// Returns `(users, total_attempts)` where `total_attempts` sums every
    /// real Composio call this fetch made across pages and rate-limit
    /// retries, so the caller can charge the daily quota meter
    /// accurately. Pages walked silently are tracked too — without this,
    /// large workspaces under-report their request usage.
    pub async fn fetch(client: &ComposioClient) -> (Self, u32) {
        let mut map: HashMap<String, String> = HashMap::new();
        let mut cursor: Option<String> = None;
        let mut total_attempts: u32 = 0;

        for page_num in 0..MAX_PAGES {
            let mut args = json!({ "limit": PAGE_SIZE });
            if let Some(ref c) = cursor {
                args["cursor"] = json!(c);
            }

            // Going through `execute_with_retry` so a transient
            // `ratelimited` page doesn't drop us into a half-built
            // directory while the rest of the provider uses backoff.
            // Soft-fall to whatever was collected so far on any failure.
            let (resp, attempts) = match super::provider::execute_with_retry(
                client,
                ACTION_LIST_USERS,
                args,
                &format!("{ACTION_LIST_USERS} page {page_num}"),
            )
            .await
            {
                Ok(t) => t,
                Err(err) => {
                    // We don't know exactly how many attempts the helper
                    // burned before bailing, but at least one ran — count
                    // it so the budget meter doesn't silently undercount.
                    total_attempts = total_attempts.saturating_add(1);
                    log::warn!(
                        "[composio:slack:users] {ACTION_LIST_USERS} page {page_num} failed: {err} — \
                         degrading to raw ids for the rest of this sync"
                    );
                    return (Self { map }, total_attempts);
                }
            };
            total_attempts = total_attempts.saturating_add(attempts);

            super::provider::dump_response("_meta", "users", page_num, &resp.data);
            absorb_page(&resp.data, &mut map);

            cursor = extract_next_cursor(&resp.data);
            if cursor.is_none() {
                break;
            }
        }

        log::info!(
            "[composio:slack:users] resolved {} workspace users in {total_attempts} call(s)",
            map.len()
        );
        (Self { map }, total_attempts)
    }

    /// Construct from a pre-built map. Test-only — production callers
    /// should use [`Self::fetch`] or [`Self::empty`].
    #[cfg(test)]
    pub fn from_map(map: HashMap<String, String>) -> Self {
        Self { map }
    }
}

/// Walk a Composio response envelope and absorb every user object's
/// `id` + best-available display name into `map`.
fn absorb_page(data: &Value, map: &mut HashMap<String, String>) {
    let candidates = [
        data.pointer("/data/members"),
        data.pointer("/members"),
        data.pointer("/data/users"),
        data.pointer("/users"),
        data.pointer("/data/data/members"),
    ];
    let arr = candidates
        .into_iter()
        .flatten()
        .find_map(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for raw in arr {
        let id = raw
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if id.is_empty() {
            continue;
        }
        if let Some(name) = pick_display_name(&raw) {
            map.insert(id, name);
        }
    }
}

/// Slack returns several name fields per user. Prefer the most
/// human-readable, fall back through real_name → name → display_name.
fn pick_display_name(raw: &Value) -> Option<String> {
    let candidates = [
        raw.pointer("/profile/display_name"),
        raw.pointer("/profile/real_name"),
        raw.get("real_name"),
        raw.get("name"),
        raw.pointer("/profile/display_name_normalized"),
        raw.pointer("/profile/real_name_normalized"),
    ];
    for cand in candidates.into_iter().flatten() {
        if let Some(s) = cand.as_str() {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn extract_next_cursor(data: &Value) -> Option<String> {
    let candidates = [
        data.pointer("/data/response_metadata/next_cursor"),
        data.pointer("/response_metadata/next_cursor"),
        data.pointer("/data/next_cursor"),
        data.pointer("/next_cursor"),
    ];
    for cand in candidates.into_iter().flatten() {
        if let Some(s) = cand.as_str() {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_users() -> SlackUsers {
        let mut m = HashMap::new();
        m.insert("U001".to_string(), "alice".to_string());
        m.insert("U002".to_string(), "bob".to_string());
        SlackUsers::from_map(m)
    }

    #[test]
    fn resolve_known_id_returns_name() {
        let u = sample_users();
        assert_eq!(u.resolve("U001"), "alice");
    }

    #[test]
    fn resolve_unknown_id_passes_through() {
        let u = sample_users();
        assert_eq!(u.resolve("U999"), "U999");
    }

    #[test]
    fn empty_passes_through_every_id() {
        let u = SlackUsers::empty();
        assert_eq!(u.resolve("U001"), "U001");
        assert_eq!(u.replace_mentions("hi <@U001>"), "hi @U001");
    }

    #[test]
    fn replace_mentions_substitutes_known_ids() {
        let u = sample_users();
        let out = u.replace_mentions("Hi <@U001>, please ping <@U002>.");
        assert_eq!(out, "Hi @alice, please ping @bob.");
    }

    #[test]
    fn replace_mentions_strips_wrapper_for_unknown_id() {
        let u = sample_users();
        // Unknown id keeps the raw id but loses the `<@...>` wrapper.
        let out = u.replace_mentions("ping <@U999>");
        assert_eq!(out, "ping @U999");
    }

    #[test]
    fn replace_mentions_leaves_non_mention_text_alone() {
        let u = sample_users();
        let out = u.replace_mentions("no mentions here, just <text>");
        assert_eq!(out, "no mentions here, just <text>");
    }

    #[test]
    fn replace_mentions_handles_multiple_in_one_line() {
        let u = sample_users();
        let out = u.replace_mentions("<@U001> said hi to <@U001> and <@U002>");
        assert_eq!(out, "@alice said hi to @alice and @bob");
    }

    #[test]
    fn absorb_page_reads_data_members_path() {
        let data = json!({
            "data": {
                "members": [
                    {
                        "id": "U001",
                        "profile": { "display_name": "alice", "real_name": "Alice Smith" }
                    },
                    {
                        "id": "U002",
                        "profile": { "display_name": "" , "real_name": "Bob Jones" }
                    },
                    {
                        "id": "",
                        "profile": { "display_name": "skipped" }
                    }
                ]
            }
        });
        let mut m = HashMap::new();
        absorb_page(&data, &mut m);
        assert_eq!(m.get("U001").unwrap(), "alice");
        // Falls back to real_name when display_name is blank.
        assert_eq!(m.get("U002").unwrap(), "Bob Jones");
        // Empty id row is dropped.
        assert!(!m.contains_key(""));
    }

    #[test]
    fn pick_display_name_prefers_display_name_over_real_name() {
        let raw = json!({
            "profile": { "display_name": "alice", "real_name": "Alice Smith" }
        });
        assert_eq!(pick_display_name(&raw).as_deref(), Some("alice"));
    }

    #[test]
    fn pick_display_name_falls_back_to_name() {
        let raw = json!({ "name": "alice", "profile": {} });
        assert_eq!(pick_display_name(&raw).as_deref(), Some("alice"));
    }

    #[test]
    fn pick_display_name_returns_none_when_all_blank() {
        let raw = json!({ "profile": { "display_name": "  " }, "name": "" });
        assert!(pick_display_name(&raw).is_none());
    }

    #[test]
    fn extract_next_cursor_finds_response_metadata() {
        let data = json!({"data": {"response_metadata": {"next_cursor": "abc123"}}});
        assert_eq!(extract_next_cursor(&data).as_deref(), Some("abc123"));
    }

    #[test]
    fn extract_next_cursor_none_when_blank() {
        let data = json!({"response_metadata": {"next_cursor": "  "}});
        assert!(extract_next_cursor(&data).is_none());
    }
}
