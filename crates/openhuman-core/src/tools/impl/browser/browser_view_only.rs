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

/// `browser` actions that change a page, act for the signed-in account, run a
/// task (which acts) or take a file onto the host.
const ACTING: &[&str] = &[
    "click",
    "double_click",
    "fill",
    "type",
    "press",
    "select",
    "check",
    "task",
    "task_continue",
    "confirm_pending",
    "wait_download",
];

/// What `find` may do with the element it found that acts on the page.
const ACTING_FIND: &[&str] = &["click", "fill"];

/// Prefix that keeps a view-only session's pool key apart from every key a
/// normal session uses (those start with the agent scope's absolute path).
const VIEW_ONLY_KEY_PREFIX: &str = "view-only:";

/// The tool error a view-only run gets for `action`, or `None` when the action
/// is left to normal handling (allowed, or unknown and refused there).
pub(super) fn view_only_refusal(action: &str, args: &Value) -> Option<String> {
    let acting = ACTING.contains(&action)
        || (action == "find"
            && args["find_action"].as_str().is_some_and(|follow| {
                ACTING_FIND.contains(&follow.trim().to_ascii_lowercase().as_str())
            }));
    acting.then(|| {
        format!(
            "{} Browser action `{action}` refused: this agent's browser is view-only \
             (browser_view_only). It may open, read, scroll and look at pages, but not \
             click, type, submit, run tasks or download.",
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
