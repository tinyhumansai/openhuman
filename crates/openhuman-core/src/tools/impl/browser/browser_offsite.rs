//! A click (or a key, or a redirect) that took the session to another site.
//!
//! A browser session is bound to the host tree of the page it was opened on
//! (`BrowserClient::open_session_for_url`). When an action on that page leads
//! somewhere else, TinyComputer leaves the page it reached and answers
//! `BlockedByPolicy` (its `LeftRefusedPage`): the action ran, the session did
//! not follow. Returned as a tool failure, that read as a missing permission
//! and ended the whole turn, so a browsing agent that clicked a link to
//! another site never got to report. The tool answers it as what it is
//! instead: the action ran, the page it led to is on another site, and the
//! agent may open that page itself with `open` (which starts a session bound
//! to that site, under every rule `open` has). The tool never follows on its
//! own: where the agent may go is still its decision.

use serde_json::{json, Value};

use crate::modules::browser::BrowserCallError;

const LEFT_PREFIX: &str = "the page reached, ";
const LEFT_SUFFIX: &str =
    ", is not permitted by this session's allowed origins; the session left it";

/// The page an action led to on another site, when `error` is
/// TinyComputer's report of leaving it and the page is a public https page.
pub(super) fn left_for(error: &BrowserCallError) -> Option<String> {
    let BrowserCallError::Bus { name, message } = error else {
        return None;
    };
    if name != "BlockedByPolicy" {
        return None;
    }
    let url = message
        .trim()
        .strip_prefix(LEFT_PREFIX)?
        .strip_suffix(LEFT_SUFFIX)?
        .trim();
    if !crate::modules::browser::is_public_https(url) {
        return None;
    }
    // The normalized form, as the browser's own URL policy reads it.
    url::Url::parse(url).ok().map(|parsed| parsed.as_str().to_owned())
}

/// What the tool answers for an action of `kind` that led to `url`.
pub(super) fn left_reply(kind: &str, url: &str) -> Value {
    json!({
        "ran": true,
        "action": kind,
        "left_for_another_site": url,
        "next": format!(
            "The {kind} ran and led to {url} on another site, which this browser session \
             does not show. Going there is a separate decision under your task's rules: \
             a host on the never-open list is refused by the browser. Otherwise carry on \
             from the page you were on."
        ),
    })
}

#[cfg(test)]
#[path = "browser_offsite_tests.rs"]
mod tests;
