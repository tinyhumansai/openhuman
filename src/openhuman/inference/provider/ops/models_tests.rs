use super::{
    managed_401_means_signed_out, managed_session_attaches, resolve_local_runtime_key,
    synthesize_managed_entry, url_is_credential_safe,
};
use crate::openhuman::config::Config;

#[test]
fn omlx_key_falls_back_to_local_ai_api_key() {
    let mut config = Config::default();
    config.local_ai.api_key = Some("  sk-omlx-list  ".into());
    assert_eq!(
        resolve_local_runtime_key("omlx", String::new(), &config),
        "sk-omlx-list"
    );
}

#[test]
fn looked_up_key_wins_over_local_ai() {
    let mut config = Config::default();
    config.local_ai.api_key = Some("sk-local".into());
    assert_eq!(
        resolve_local_runtime_key("omlx", "from-profiles".into(), &config),
        "from-profiles"
    );
}

#[test]
fn non_omlx_slug_does_not_fall_back() {
    let mut config = Config::default();
    config.local_ai.api_key = Some("sk-local".into());
    assert_eq!(
        resolve_local_runtime_key("ollama", String::new(), &config),
        ""
    );
}

/// A bearer credential must never ride a plaintext connection to a remote host.
/// Loopback stays allowed so a locally-hosted backend still authenticates in
/// development.
#[test]
fn credentials_ride_https_or_loopback_only() {
    for url in [
        "https://api.tinyhumans.ai/openai/v1/models",
        "https://staging-api.tinyhumans.ai/openai/v1/models?catalog=openrouter",
        "http://localhost:5005/openai/v1/models",
        "http://127.0.0.1:5005/openai/v1/models",
    ] {
        assert!(url_is_credential_safe(url), "{url}");
    }
}

#[test]
fn credentials_are_withheld_from_remote_plaintext_and_junk_urls() {
    for url in [
        "http://api.tinyhumans.ai/openai/v1/models",
        "http://192.168.1.10:5005/openai/v1/models",
        "not a url",
        "",
    ] {
        assert!(!url_is_credential_safe(url), "{url}");
    }
}

/// The managed row is normally seeded by a migration, but a freshly created
/// profile has none — which is what the app falls back to when a stored session
/// is rejected. Managed is the product's own backend, so it must not depend on a
/// user-config row; without this, being signed out surfaced
/// "no cloud provider with id or slug 'openhuman' found".
#[test]
fn managed_entry_is_synthesized_when_the_config_row_is_missing() {
    use crate::openhuman::config::schema::cloud_providers::AuthStyle;
    let entry = synthesize_managed_entry("openhuman").expect("managed entry");
    assert_eq!(entry.slug, "openhuman");
    assert_eq!(entry.auth_style, AuthStyle::OpenhumanJwt);
    assert!(!entry.endpoint.is_empty());
}

/// Only the managed slug synthesizes: anything else must keep reporting an
/// unknown provider rather than being silently treated as managed.
#[test]
fn other_slugs_do_not_synthesize_a_managed_entry() {
    for slug in ["openai", "openrouter", "ollama", "", "openhuman-x"] {
        assert!(synthesize_managed_entry(slug).is_none(), "{slug}");
    }
}

/// A managed 401 is a signed-out state, not a provider failure: the stored
/// session was rejected server-side while its local `exp` was still valid, so
/// the picker rendered "could not load models" for a user whose only problem
/// was a stale session.
#[test]
fn a_managed_401_reads_as_signed_out() {
    use crate::openhuman::config::schema::cloud_providers::AuthStyle;
    assert!(managed_401_means_signed_out(
        401,
        AuthStyle::OpenhumanJwt,
        true
    ));
}

/// When no session existed, the request went out with the provider-scoped
/// fallback key — so a 401 means THAT key is wrong or revoked. Hiding it behind
/// an empty catalog would strand a self-hosted entry with no clue why.
#[test]
fn a_401_against_the_fallback_key_still_surfaces() {
    use crate::openhuman::config::schema::cloud_providers::AuthStyle;
    assert!(!managed_401_means_signed_out(
        401,
        AuthStyle::OpenhumanJwt,
        false
    ));
}

/// A live session is not enough on its own: the credential-safety guard refuses
/// to attach a bearer token to a non-https, non-loopback URL, so the request
/// goes out unauthenticated. Reporting that 401 as signed-out would hide the
/// misconfigured backend URL behind an empty catalog — the same failure this
/// whole change exists to stop.
#[test]
fn a_session_is_not_attached_to_an_unsafe_url() {
    for url in [
        "http://api.example.com/openai/v1/models",
        "http://192.168.1.10:8080/openai/v1/models",
    ] {
        assert!(
            !managed_session_attaches("session-jwt", url),
            "a live session must not be attached to {url}"
        );
    }
    for url in [
        "https://api.tinyhumans.ai/openai/v1/models",
        "http://127.0.0.1:8787/openai/v1/models",
    ] {
        assert!(
            managed_session_attaches("session-jwt", url),
            "a live session must be attached to {url}"
        );
    }
    // No session: nothing to attach, whatever the URL.
    assert!(!managed_session_attaches(
        "",
        "https://api.tinyhumans.ai/openai/v1/models"
    ));
}

/// The two halves composed: a live session withheld by the safety guard reaches
/// the 401 classifier as "not attached", so the error still surfaces.
#[test]
fn a_401_on_a_token_the_safety_guard_withheld_still_surfaces() {
    use crate::openhuman::config::schema::cloud_providers::AuthStyle;
    let attached = managed_session_attaches("session-jwt", "http://api.example.com/v1/models");
    assert!(!managed_401_means_signed_out(
        401,
        AuthStyle::OpenhumanJwt,
        attached
    ));
}

/// Everything else must keep surfacing the error. A BYOK 401 is the actionable
/// case (wrong or revoked key) and must not be swallowed into an empty list,
/// and a managed non-401 is a genuine provider failure.
#[test]
fn other_statuses_and_providers_still_surface_the_error() {
    use crate::openhuman::config::schema::cloud_providers::AuthStyle;
    for style in [AuthStyle::Bearer, AuthStyle::Anthropic, AuthStyle::None] {
        assert!(
            !managed_401_means_signed_out(401, style, true),
            "{style:?} 401"
        );
    }
    for status in [400, 403, 404, 429, 500, 503] {
        assert!(
            !managed_401_means_signed_out(status, AuthStyle::OpenhumanJwt, true),
            "managed {status}"
        );
    }
}
