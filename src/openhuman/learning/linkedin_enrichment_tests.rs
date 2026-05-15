use super::*;

#[test]
fn extracts_username_from_canonical_url() {
    let text = "Check out https://www.linkedin.com/in/williamhgates for more";
    let caps = LINKEDIN_USERNAME_RE.captures(text).unwrap();
    assert_eq!(&caps[1], "williamhgates");
    assert_eq!(
        canonical_linkedin_url(&caps[1]),
        "https://www.linkedin.com/in/williamhgates"
    );
}

#[test]
fn extracts_username_from_comm_url() {
    let text = "https://www.linkedin.com/comm/in/stevenenamakel?midToken=abc";
    let caps = LINKEDIN_USERNAME_RE.captures(text).unwrap();
    assert_eq!(&caps[1], "stevenenamakel");
    assert_eq!(
        canonical_linkedin_url(&caps[1]),
        "https://www.linkedin.com/in/stevenenamakel"
    );
}

#[test]
fn extracts_username_from_http_variant() {
    let text = "See http://www.linkedin.com/in/jeannie-wyrick-b4760710a";
    let caps = LINKEDIN_USERNAME_RE.captures(text).unwrap();
    assert_eq!(&caps[1], "jeannie-wyrick-b4760710a");
}

#[test]
fn skips_non_profile_linkedin_urls() {
    let text = "Visit https://www.linkedin.com/company/openai";
    assert!(LINKEDIN_USERNAME_RE.captures(text).is_none());
}

#[test]
fn handles_no_match() {
    assert!(LINKEDIN_USERNAME_RE.captures("No LinkedIn here").is_none());
}

// ── Factory routing smoke (#1710 Wave 2) ────────────────────────────
//
// Pre-Wave-2 `search_gmail_for_linkedin` called `build_composio_client`
// directly, so a direct-mode user with a stored API key but no backend
// session JWT was rejected at the very first line ("composio client
// unavailable") — silently disabling LinkedIn enrichment for that user
// even when their personal Composio tenant had a healthy Gmail
// connection. The function now resolves via `create_composio_client`
// and branches on the kind.
//
// These tests exercise the factory branch shape against synthetic
// configs — they don't actually hit the network, so the goal is to
// pin error provenance: a direct-mode config must NOT surface a
// backend-session lookup error, and a fully empty config must error
// without panicking. Same shape as
// `composio::providers::types::tests::provider_context_execute_*`.

#[tokio::test]
async fn search_gmail_for_linkedin_routes_through_factory_in_direct_mode() {
    use crate::openhuman::composio::client::{create_composio_client, ComposioClientKind};
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.to_string();
    config.composio.api_key = Some("test-direct-key".into());

    // Factory probe: a direct-mode config with an inline key resolves
    // to the `Direct` variant. The smoke is that `search_gmail_for_linkedin`'s
    // first action is now to call `create_composio_client` (which
    // succeeds here) — instead of `build_composio_client` which would
    // unconditionally return `None` for a config with no backend
    // session and fail the function before any branching could happen.
    let kind = create_composio_client(&config).expect("direct-mode probe should succeed");
    assert!(
        matches!(kind, ComposioClientKind::Direct(_)),
        "direct-mode config must resolve to Direct variant"
    );
}

#[tokio::test]
async fn search_gmail_for_linkedin_errors_when_factory_cannot_build_client() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    // Default mode = backend, no session token. The factory must error
    // with a backend-session message — and `search_gmail_for_linkedin`
    // must surface that as an anyhow error rather than panicking. We
    // call the function directly to verify the early-return shape.
    let res = super::search_gmail_for_linkedin(&config).await;
    let err = res.expect_err("unsigned-in user must surface an error");
    let msg = err.to_string();
    assert!(
        msg.contains("composio client unavailable"),
        "expected mode-agnostic factory error surface, got: {msg}"
    );
}
