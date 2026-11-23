//! Which profile folder a TinyComputer browser session opens on.

use crate::config::BrowserConfig;

/// The profile folder a session asks for: `[browser] profile_path` under the
/// persistent profile mode, unless the client opens on a fresh profile
/// (`BrowserClient::with_fresh_profile`); `None` is a fresh, throwaway one.
pub(super) fn session_profile(browser: &BrowserConfig, fresh_profile: bool) -> Option<String> {
    if fresh_profile || browser.profile_mode != "persistent" {
        return None;
    }
    browser.profile_path.clone()
}

#[cfg(test)]
#[path = "browser_profile_tests.rs"]
mod tests;
