use super::*;
use openhuman_embed::process::sentry::known_noise;

fn channel_message_404_event(method: &str) -> Event<'static> {
    let mut event = Event::default();
    event.tags.insert("domain".into(), "backend_api".into());
    event.tags.insert("failure".into(), "non_2xx".into());
    event.tags.insert("status".into(), "404".into());
    event.tags.insert("method".into(), method.into());
    event.message = Some(
        "PATCH /channels/telegram/messages/1103 failed (404); response_body_len=172".to_string(),
    );
    event
}

#[test]
fn channel_message_not_found_filter_matches_patch() {
    // Canonical TAURI-R7 shape: PATCH 404 on a channel-message path.
    assert!(is_channel_message_not_found_event(
        &channel_message_404_event("PATCH")
    ));
}

#[test]
fn channel_message_not_found_filter_matches_delete() {
    assert!(is_channel_message_not_found_event(
        &channel_message_404_event("DELETE")
    ));
}

#[test]
fn channel_message_not_found_filter_ignores_get_404() {
    // GET 404 on a channel-message path is NOT an expected state — must keep Sentry signal.
    assert!(!is_channel_message_not_found_event(
        &channel_message_404_event("GET")
    ));
}

#[test]
fn channel_message_not_found_filter_ignores_non_channel_path() {
    let mut event = channel_message_404_event("PATCH");
    event.message = Some("PATCH /auth/profile failed (404); response_body_len=42".to_string());
    assert!(!is_channel_message_not_found_event(&event));
}

#[test]
fn channel_message_not_found_filter_ignores_wrong_status() {
    let mut event = channel_message_404_event("PATCH");
    event.tags.insert("status".into(), "403".into());
    assert!(!is_channel_message_not_found_event(&event));
}

#[test]
fn channel_message_not_found_filter_ignores_wrong_domain() {
    let mut event = channel_message_404_event("PATCH");
    event.tags.insert("domain".into(), "channels".into());
    assert!(!is_channel_message_not_found_event(&event));
}

#[test]
fn channel_message_not_found_filter_matches_exception_path() {
    // sentry-tracing with attach_stacktrace=true populates exception list.
    let mut event = Event::default();
    event.tags.insert("domain".into(), "backend_api".into());
    event.tags.insert("failure".into(), "non_2xx".into());
    event.tags.insert("status".into(), "404".into());
    event.tags.insert("method".into(), "PATCH".into());
    event.exception = vec![openhuman_embed::process::sentry::sdk::protocol::Exception {
        value: Some("PATCH /channels/discord/messages/abc failed (404): Not Found".to_string()),
        ..Default::default()
    }]
    .into();
    assert!(is_channel_message_not_found_event(&event));
}

#[test]
fn registered_filter_drops_the_event_in_the_shared_chain() {
    register();
    assert_eq!(
        known_noise(&channel_message_404_event("DELETE")),
        Some(FILTER_NAME)
    );
    assert_eq!(known_noise(&channel_message_404_event("GET")), None);
}
