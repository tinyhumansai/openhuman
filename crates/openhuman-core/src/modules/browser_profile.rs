//! Stub.

use crate::config::BrowserConfig;

pub(super) fn session_profile(browser: &BrowserConfig, _fresh_profile: bool) -> Option<String> {
    if browser.profile_mode == "persistent" {
        browser.profile_path.clone()
    } else {
        None
    }
}

#[cfg(test)]
#[path = "browser_profile_tests.rs"]
mod tests;
