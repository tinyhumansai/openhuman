//! The view-only browser an agent definition can ask for
//! (`AgentExt::browser_view_only`): which `browser` actions such a run is
//! refused, and the pool key its own sessions live under.
//!
//! A view-only run may open, read, look at and scroll pages. It may not change
//! a page or act for whoever the browser is signed in as, and its sessions
//! open on a fresh profile (`BrowserClient::with_fresh_profile`), pooled apart
//! from every session on the persistent one. The refusal comes before the
//! approval gate and `[browser] unattended_actions`, so neither can re-allow
//! an action.

use serde_json::Value;
use std::path::Path;

/// The only `browser` actions a view-only run has: opening, reading, looking
/// and leaving. Everything else is refused, so an action added to the tool
/// later is refused until it is listed here. `find` is listed and checked
/// further by what it does with the element (`VIEWING_FIND`).
const VIEWING: &[&str] = &[
    "open",
    "snapshot",
    "read_page",
    "get_text",
    "get_title",
    "get_url",
    "wait",
    "hover",
    "scroll",
    "is_visible",
    "find",
    "list_downloads",
    "close",
];

/// What `find` may do with the element it found without acting on the page.
const VIEWING_FIND: &[&str] = &["text", "hover"];

/// Prefix that keeps a view-only session's pool key apart from every key a
/// normal session uses (those start with the agent scope's absolute path).
const VIEW_ONLY_KEY_PREFIX: &str = "view-only:";

/// The tool error a view-only run gets for `action`, or `None` when it is one
/// of the viewing actions. An action this list does not know is refused: that
/// covers every acting action, tasks (which act, and whose ids are not this
/// run's to cancel), downloads, and anything misspelt.
pub(super) fn view_only_refusal(action: &str, args: &Value) -> Option<String> {
    let viewing = VIEWING.contains(&action)
        && (action != "find"
            || args["find_action"]
                .as_str()
                .is_some_and(|follow| VIEWING_FIND.contains(&follow)));
    let acting = !viewing;
    acting.then(|| {
        format!(
            "{} Browser action `{action}` refused: this agent's browser is view-only \
             (browser_view_only). It may open, read, scroll and look at pages, but not \
             click, type, submit, run or cancel tasks, or download.",
            tinytools::status::POLICY_BLOCKED_MARKER
        )
    })
}

/// The key a conversation's pooled session lives under: the agent scope and
/// thread, plus a prefix for a view-only run, so a view-only run never picks
/// up a session opened on the persistent profile, nor a normal run one of its.
pub(super) fn pool_key(scope: &Path, thread_id: &str, view_only: bool) -> String {
    let key = format!("{}:{thread_id}", scope.display());
    if view_only {
        format!("{VIEW_ONLY_KEY_PREFIX}{key}")
    } else {
        key
    }
}

#[cfg(test)]
#[path = "browser_view_only_tests.rs"]
mod tests;
