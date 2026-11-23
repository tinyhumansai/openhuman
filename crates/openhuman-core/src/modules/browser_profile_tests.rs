use super::*;
use crate::config::Config;
use crate::modules::browser::BrowserClient;
use std::sync::Arc;

fn persistent() -> BrowserConfig {
    BrowserConfig {
        profile_mode: "persistent".into(),
        profile_path: Some("/home/host/.config/teeny-chrome".into()),
        ..BrowserConfig::default()
    }
}

#[test]
fn a_normal_session_uses_the_configured_persistent_profile() {
    assert_eq!(
        session_profile(&persistent(), false).as_deref(),
        Some("/home/host/.config/teeny-chrome")
    );
}

#[test]
fn a_fresh_profile_session_never_uses_the_persistent_profile() {
    assert_eq!(session_profile(&persistent(), true), None);
}

#[test]
fn a_fresh_profile_mode_uses_no_profile_either_way() {
    let fresh = BrowserConfig {
        profile_mode: "fresh".into(),
        profile_path: Some("/home/host/.config/teeny-chrome".into()),
        ..BrowserConfig::default()
    };
    assert_eq!(session_profile(&fresh, false), None);
    assert_eq!(session_profile(&fresh, true), None);
}

/// The client a view-only run uses resolves to a fresh profile under the same
/// configuration that gives every other session the persistent one.
#[test]
fn a_fresh_profile_client_drops_the_persistent_profile_and_keeps_the_rest() {
    let mut config = Config::default();
    config.browser = persistent();
    let client = BrowserClient::new(Arc::new(config));
    assert_eq!(
        client.session_profile().as_deref(),
        Some("/home/host/.config/teeny-chrome")
    );
    let fresh = client.with_fresh_profile();
    assert_eq!(fresh.session_profile(), None);
    assert_eq!(fresh.config().browser.profile_mode, "persistent");
    assert_eq!(
        client.session_profile().as_deref(),
        Some("/home/host/.config/teeny-chrome"),
        "the original client is untouched"
    );
}
