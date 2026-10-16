use super::*;

#[test]
fn a_managed_401_on_an_attached_session_reads_as_signed_out() {
    assert!(managed_401_means_signed_out(401, true));
}

/// A 401 on a request that never carried the session token describes the
/// request (a misconfigured URL), not the session, so the error surfaces.
#[test]
fn a_401_without_an_attached_session_still_surfaces() {
    assert!(!managed_401_means_signed_out(401, false));
}

#[test]
fn other_statuses_still_surface_the_error() {
    for status in [400, 403, 404, 429, 500, 503] {
        assert!(
            !managed_401_means_signed_out(status, true),
            "managed {status}"
        );
    }
}

#[test]
fn the_catalog_url_targets_the_openrouter_catalog_on_the_control_plane() {
    let config = Config {
        api_url: Some("https://api.tinyhumans.ai".into()),
        ..Config::default()
    };
    let url = catalog_url(&config);
    assert!(
        url.ends_with("/openai/v1/models?catalog=openrouter"),
        "{url}"
    );
}

#[test]
fn limits_request_is_none_for_a_blank_model() {
    assert!(limits_request("  ", &Config::default()).is_none());
}

#[test]
fn limits_request_lists_the_catalog_keyed_by_the_inference_base() {
    let config = Config {
        api_url: Some("https://api.tinyhumans.ai".into()),
        ..Config::default()
    };
    let request = limits_request("some/model", &config).expect("managed limits request");
    assert_eq!(request.endpoint, "https://api.tinyhumans.ai/openai/v1");
    assert!(request
        .listing_url
        .as_deref()
        .is_some_and(|url| url.ends_with("/openai/v1/models?catalog=openrouter")));
}

#[test]
fn truncate_caps_long_text_with_an_ellipsis() {
    assert_eq!(truncate("abc", 5), "abc");
    assert_eq!(truncate("abcdef", 3), "abc...");
}

#[test]
fn a_live_session_wins_over_the_provider_key() {
    assert_eq!(
        listing_auth(SessionTokenCheck::Live("jwt".into()), "provider-key"),
        ListingAuth::Session("jwt".into())
    );
}

/// A self-hosted entry that stores its own key still lists its catalog when the
/// user is signed out; before the move this fell back to that key.
#[test]
fn without_a_live_session_the_entrys_provider_key_authenticates() {
    assert_eq!(
        listing_auth(SessionTokenCheck::Absent, " provider-key "),
        ListingAuth::ProviderKey("provider-key".into())
    );
    assert_eq!(
        listing_auth(SessionTokenCheck::Expired, "provider-key"),
        ListingAuth::ProviderKey("provider-key".into())
    );
}

#[test]
fn with_neither_credential_the_catalog_is_empty() {
    assert_eq!(
        listing_auth(SessionTokenCheck::Absent, ""),
        ListingAuth::Empty("no session")
    );
    assert_eq!(
        listing_auth(SessionTokenCheck::Expired, "  "),
        ListingAuth::Empty("session expired")
    );
}
