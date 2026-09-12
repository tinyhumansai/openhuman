use super::*;

#[test]
fn is_newer_detects_update() {
    assert!(is_newer("0.50.0", "0.49.17"));
    assert!(is_newer("1.0.0", "0.99.99"));
    assert!(is_newer("v0.50.0", "0.49.17"));
    assert!(!is_newer("0.49.17", "0.49.17"));
    assert!(!is_newer("0.49.16", "0.49.17"));
    assert!(!is_newer("0.49.17", "0.50.0"));
}

#[test]
fn current_version_is_not_empty() {
    assert!(!current_version().is_empty());
}

/// OPENHUMAN-TAURI-2F regression guard. A reqwest call to an unroutable
/// host (port 1 on TEST-NET-1, RFC 5737 documentation range — guaranteed
/// never to answer) must classify as a transport failure so the
/// `check_releases` / `download` call sites skip the Sentry report. If
/// reqwest ever changes its error taxonomy and connection failures stop
/// setting `is_connect` / `is_request` / `is_timeout`, this test breaks
/// and the call sites would silently start paging again — that's the
/// signal we want.
#[tokio::test]
async fn transport_failure_classifier_catches_unreachable_host() {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(250))
        .no_proxy()
        .build()
        .expect("build reqwest client");
    let result = client.get("http://192.0.2.1:1/").send().await;
    let err = result.expect_err("connect to TEST-NET-1:1 must fail");
    assert!(
        is_transport_network_failure(&err),
        "unreachable-host reqwest error must classify as transport: {err}"
    );
}

// ---------------------------------------------------------------------------
// #6089: `check_available` used to build `https://api.github.com/...` inline, so
// the HTTP + JSON-parse path could only be exercised against the live endpoint —
// which `scheduler_tests.rs` calls out as the reason `tick()` is untested. These
// drive the private `check_available_with_base_url` seam against a local mock.
// ---------------------------------------------------------------------------

/// The release JSON GitHub actually returns, trimmed to the fields we parse.
/// `decoy` is an asset for another platform: it must never be selected.
fn release_body(tag: &str) -> String {
    let triple = platform_triple();
    format!(
        r#"{{
            "tag_name": "{tag}",
            "body": "release notes for {tag}",
            "published_at": "2026-09-08T00:00:00Z",
            "assets": [
                {{"name": "openhuman-core-some-other-triple", "browser_download_url": "https://example.invalid/decoy", "size": 1}},
                {{"name": "openhuman-core-{triple}", "browser_download_url": "https://example.invalid/{triple}", "size": 2}}
            ]
        }}"#
    )
}

async fn releases_mock(status: u16, body: &str) -> wiremock::MockServer {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/tinyhumansai/openhuman/releases/latest"))
        .respond_with(ResponseTemplate::new(status).set_body_string(body))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn check_available_parses_a_release_and_picks_this_platforms_asset() {
    let server = releases_mock(200, &release_body("v99.0.0")).await;

    let info = check_available_with_base_url(&server.uri())
        .await
        .expect("mocked release must parse");

    assert_eq!(info.latest_version, "99.0.0", "the leading v is stripped");
    assert_eq!(info.current_version, current_version());
    assert!(
        info.update_available,
        "99.0.0 is newer than {}",
        current_version()
    );
    assert_eq!(
        info.asset_name.as_deref(),
        Some(format!("openhuman-core-{}", platform_triple()).as_str()),
        "the decoy asset for another triple must not be selected"
    );
    assert_eq!(
        info.download_url.as_deref(),
        Some(format!("https://example.invalid/{}", platform_triple()).as_str())
    );
    assert_eq!(
        info.release_notes.as_deref(),
        Some("release notes for v99.0.0")
    );
    assert_eq!(info.published_at.as_deref(), Some("2026-09-08T00:00:00Z"));
}

#[tokio::test]
async fn check_available_reports_no_update_for_an_older_tag() {
    let server = releases_mock(200, &release_body("v0.0.1")).await;

    let info = check_available_with_base_url(&server.uri())
        .await
        .expect("mocked release must parse");

    assert_eq!(info.latest_version, "0.0.1");
    assert!(
        !info.update_available,
        "0.0.1 must not be newer than {}",
        current_version()
    );
}

#[tokio::test]
async fn check_available_surfaces_a_non_2xx_as_a_github_api_error() {
    // 403 is what the unauthenticated rate limit returns — the case the issue
    // says a caller currently cannot tell apart from "no update".
    let server = releases_mock(403, r#"{"message":"API rate limit exceeded"}"#).await;

    let err = check_available_with_base_url(&server.uri())
        .await
        .expect_err("a 403 must not be reported as a successful check");

    assert!(
        err.contains("GitHub API error: 403"),
        "error must name the status, got: {err}"
    );
}

#[tokio::test]
async fn check_available_surfaces_a_malformed_body_as_a_parse_error() {
    let server = releases_mock(200, "{ not json").await;

    let err = check_available_with_base_url(&server.uri())
        .await
        .expect_err("a malformed body must not be reported as a successful check");

    assert!(
        err.contains("failed to parse release JSON"),
        "error must distinguish parse from transport, got: {err}"
    );
}

/// The seam exists for tests only: the shipped binary must still resolve the
/// pinned GitHub origin. Guards the extraction itself — if someone later makes
/// the base URL configurable at runtime, this is what breaks, and
/// `ops::validate_download_url`'s host allowlist is why that matters.
#[test]
fn the_default_origin_is_still_the_pinned_github_api() {
    assert_eq!(GITHUB_API_BASE, "https://api.github.com");
    assert_eq!(
        format!("{GITHUB_API_BASE}/repos/{GITHUB_OWNER}/{GITHUB_REPO}/releases/latest"),
        "https://api.github.com/repos/tinyhumansai/openhuman/releases/latest",
        "the URL check_available() builds must be byte-identical to the pre-refactor literal"
    );
}
