//! Tests for the core's binding of release staging.

use super::*;

#[test]
fn current_version_is_not_empty() {
    assert!(!current_version().is_empty());
}

#[test]
fn core_releases_keep_the_published_names() {
    assert_eq!(CORE_RELEASES.owner, "tinyhumansai");
    assert_eq!(CORE_RELEASES.repo, "openhuman");
    assert_eq!(CORE_RELEASES.asset_prefix, "openhuman-core");
}

/// A refused download reaches the caller as an error, and nothing is staged.
#[tokio::test]
async fn staging_without_a_checksum_sidecar_is_refused() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let asset = format!(
        "openhuman-core-{}",
        openhuman_platform::update::platform_triple()
    );
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/{asset}.sha256")))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let dir = tempfile::TempDir::new().expect("temp dir");

    let err = download_and_stage_with_version(
        &format!("{}/{asset}", server.uri()),
        &asset,
        Some(dir.path().to_path_buf()),
        Some("99.0.0"),
        None,
    )
    .await
    .expect_err("an unverifiable update must be refused");

    assert!(err.contains("checksum sidecar unavailable"), "{err}");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
