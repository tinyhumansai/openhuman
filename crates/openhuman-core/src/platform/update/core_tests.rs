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

/// Sentry TAURI-RUST-122R/122S/13B8/13B9: releases publish core archives for
/// Linux only — macOS and Windows update through the Tauri updater (DMG/MSI) —
/// so on those platforms every scheduled check found "no core asset" and
/// reported an error. A newer release without a core asset for this triple is
/// an available update with nothing to download, not a failure.
#[cfg(not(target_os = "linux"))]
#[test]
fn check_available_reports_an_update_without_a_download_when_no_platform_asset_is_published() {
    // The release a macOS/Windows host sees: desktop installers plus a core
    // archive for a triple that is not this one.
    let other_triple = if platform_triple() == "x86_64-unknown-linux-gnu" {
        "aarch64-unknown-linux-gnu"
    } else {
        "x86_64-unknown-linux-gnu"
    };
    let body = format!(
        r#"{{
            "tag_name": "v99.0.0",
            "body": "notes",
            "published_at": "2026-09-29T00:00:00Z",
            "assets": [
                {{"name": "OpenHuman_99.0.0_aarch64.dmg", "browser_download_url": "https://example.invalid/dmg", "size": 1}},
                {{"name": "OpenHuman_99.0.0_x64_en-US.msi", "browser_download_url": "https://example.invalid/msi", "size": 1}},
                {{"name": "openhuman-core-99.0.0-{other_triple}.tar.gz", "browser_download_url": "https://example.invalid/linux", "size": 1}}
            ]
        }}"#
    );

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let mut result = None;
    let events = sentry::test::with_captured_events(|| {
        result = Some(runtime.block_on(async {
            let server = releases_mock(200, &body).await;
            check_available_with_base_url(&server.uri()).await
        }));
    });

    let info = result
        .expect("check ran")
        .expect("a missing platform asset must not fail the check");
    assert!(info.update_available, "99.0.0 is still an available update");
    assert_eq!(info.latest_version, "99.0.0");
    assert!(info.download_url.is_none(), "no asset, no download url");
    assert!(info.asset_name.is_none());
    assert!(
        events.is_empty(),
        "a missing platform asset is expected, not a Sentry event: {events:?}"
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

// ---------------------------------------------------------------------------
// #6766: the release publishes the core as `openhuman-core-<version>-<triple>`
// with the version BETWEEN the prefix and the triple, so matching on
// `openhuman-core-{triple}` never hit — on any platform, which is why every
// check reported `asset=(none)`. #908 fixed this once; the pre-#908 matcher came
// back when this module moved into the core. These pin both halves of that fix:
// selecting the versioned archive, and staging the binary out of it rather than
// marking the archive itself executable.
// ---------------------------------------------------------------------------

/// The archive extension the release workflow publishes for this platform.
fn archive_ext() -> &'static str {
    if cfg!(windows) {
        ".zip"
    } else {
        ".tar.gz"
    }
}

fn asset(name: &str) -> GitHubAsset {
    GitHubAsset {
        name: name.to_string(),
        browser_download_url: format!("https://example.invalid/{name}"),
        size: 1,
    }
}

#[test]
fn find_platform_asset_matches_the_versioned_archive() {
    let triple = platform_triple();
    let ext = archive_ext();
    let wanted = format!("openhuman-core-0.64.7-{triple}{ext}");
    let assets = vec![
        asset(&format!("openhuman-core-0.64.7-some-other-triple{ext}")),
        asset(&wanted),
    ];

    let picked = find_platform_asset(&assets).expect("the versioned archive must be selected");

    assert_eq!(
        picked.name, wanted,
        "another triple's archive must not be selected"
    );
}

#[test]
fn find_platform_asset_ignores_the_checksum_and_signature_siblings() {
    // The release publishes `….sha256` and `….sig` next to the archive. Both
    // start with the prefix and contain the triple, so a looser match would
    // stage a 65-byte checksum file as the new core binary.
    let triple = platform_triple();
    let ext = archive_ext();
    let archive = format!("openhuman-core-0.64.7-{triple}{ext}");
    let assets = vec![
        asset(&format!("{archive}.sha256")),
        asset(&format!("{archive}.sig")),
        asset(&archive),
    ];

    let picked = find_platform_asset(&assets).expect("the archive itself must be selected");
    assert_eq!(picked.name, archive);

    // And with the archive absent, neither sibling is an acceptable substitute.
    let siblings_only = vec![
        asset(&format!("{archive}.sha256")),
        asset(&format!("{archive}.sig")),
    ];
    assert!(
        find_platform_asset(&siblings_only).is_none(),
        "a checksum or signature must never be selected as the binary"
    );
}

#[test]
fn find_platform_asset_falls_back_to_a_legacy_raw_binary() {
    // Older releases shipped an unversioned raw binary. Keep resolving those.
    let triple = platform_triple();
    let legacy = if cfg!(windows) {
        format!("openhuman-core-{triple}.exe")
    } else {
        format!("openhuman-core-{triple}")
    };
    let assets = vec![asset("openhuman-core-some-other-triple"), asset(&legacy)];

    let picked = find_platform_asset(&assets).expect("the legacy raw binary must still resolve");

    assert_eq!(picked.name, legacy);
}

#[test]
fn find_platform_asset_returns_none_when_no_core_asset_is_published() {
    // What macOS and Windows actually see today: the release carries desktop
    // bundles but no `openhuman-core-*` archive for them. `None` is the correct
    // answer, and `ops::validate_asset_name` is why a bundle must not be
    // substituted — it requires the `openhuman-core-` prefix.
    let assets = vec![
        asset("OpenHuman_0.64.7_aarch64-apple-darwin.app.tar.gz"),
        asset("OpenHuman_0.64.7_x64-setup.exe"),
        asset("latest.json"),
    ];

    assert!(find_platform_asset(&assets).is_none());
}

#[test]
fn is_archive_asset_classifies_the_published_shapes() {
    assert!(is_archive_asset(
        "openhuman-core-0.64.7-x86_64-unknown-linux-gnu.tar.gz"
    ));
    assert!(is_archive_asset(
        "openhuman-core-0.64.7-x86_64-pc-windows-msvc.zip"
    ));
    assert!(is_archive_asset("core.tgz"));
    assert!(!is_archive_asset("openhuman-core-x86_64-unknown-linux-gnu"));
    assert!(!is_archive_asset("openhuman-core.exe"));
}

#[tokio::test]
async fn check_available_picks_the_versioned_archive() {
    // The reported symptom, end to end through the check path: a release shaped
    // like the real one must resolve an asset instead of `asset=(none)`.
    let triple = platform_triple();
    let ext = archive_ext();
    let archive = format!("openhuman-core-0.64.7-{triple}{ext}");
    let body = format!(
        r#"{{
            "tag_name": "v99.0.0",
            "body": "notes",
            "published_at": "2026-09-29T00:00:00Z",
            "assets": [
                {{"name": "latest.json", "browser_download_url": "https://example.invalid/latest.json", "size": 1}},
                {{"name": "{archive}.sha256", "browser_download_url": "https://example.invalid/sha", "size": 65}},
                {{"name": "{archive}", "browser_download_url": "https://example.invalid/{archive}", "size": 2}}
            ]
        }}"#
    );
    let server = releases_mock(200, &body).await;

    let info = check_available_with_base_url(&server.uri())
        .await
        .expect("mocked release must parse");

    assert!(info.update_available);
    assert_eq!(
        info.asset_name.as_deref(),
        Some(archive.as_str()),
        "the versioned archive must resolve — `asset=(none)` is the bug"
    );
    assert_eq!(
        info.download_url.as_deref(),
        Some(format!("https://example.invalid/{archive}").as_str())
    );
}

/// Build a `.tar.gz` holding one entry named `entry_name`, with `body` as its
/// contents, and return its path.
#[cfg(unix)]
fn write_tar_gz(
    dir: &std::path::Path,
    archive_name: &str,
    entry_name: &str,
    body: &[u8],
) -> std::path::PathBuf {
    let archive_path = dir.join(archive_name);
    let file = std::fs::File::create(&archive_path).expect("create archive");
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
    let mut builder = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_size(body.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    builder
        .append_data(&mut header, entry_name, body)
        .expect("append entry");
    builder
        .into_inner()
        .expect("finish tar")
        .finish()
        .expect("finish gz");
    archive_path
}

#[cfg(unix)]
#[test]
fn staging_extracts_the_core_binary_out_of_the_archive() {
    // Without extraction the `.tar.gz` itself was written to the staging path
    // and marked 0755, so a restart would exec a tarball.
    let dir = tempfile::TempDir::new().expect("temp dir");
    let archive = write_tar_gz(
        dir.path(),
        "openhuman-core-0.64.7-triple.tar.gz",
        "openhuman-core",
        b"#!/bin/sh\nexit 0\n",
    );
    let dest = dir.path().join(staged_binary_name());

    extract_core_binary(&archive, &dest, false).expect("the inner binary must extract");

    assert_eq!(
        std::fs::read(&dest).expect("read staged binary"),
        b"#!/bin/sh\nexit 0\n",
        "the staged file must be the inner binary, not the archive"
    );
    let mode = std::fs::metadata(&dest)
        .expect("stat staged binary")
        .permissions()
        .mode();
    assert_eq!(
        mode & 0o111,
        0o111,
        "the staged binary must be executable, mode was {mode:o}"
    );
}

#[cfg(unix)]
#[test]
fn staging_refuses_an_archive_without_the_core_binary() {
    // A release that renames the inner file must fail loudly rather than leave a
    // half-staged update behind.
    let dir = tempfile::TempDir::new().expect("temp dir");
    let archive = write_tar_gz(
        dir.path(),
        "openhuman-core-0.64.7-triple.tar.gz",
        "something-else",
        b"not the core\n",
    );
    let dest = dir.path().join(staged_binary_name());

    let err =
        extract_core_binary(&archive, &dest, false).expect_err("a wrong-named entry must fail");

    assert!(
        err.contains(staged_binary_name()),
        "the error must name the entry it looked for, got: {err}"
    );
    assert!(
        !dest.exists(),
        "nothing may be staged when extraction fails"
    );
}

#[test]
fn staging_extracts_zip_when_archive_format_is_explicit() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let archive = dir.path().join("release.zip");
    let file = std::fs::File::create(&archive).expect("create zip");
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file(
        staged_binary_name(),
        zip::write::SimpleFileOptions::default(),
    )
    .expect("start binary entry");
    zip.write_all(b"core binary").expect("write binary entry");
    zip.finish().expect("finish zip");

    // Download staging uses a `.tmp` suffix, so decoder choice must not rely
    // on the temporary archive path's extension.
    let downloaded = dir.path().join(".release.zip.tmp");
    std::fs::copy(&archive, &downloaded).expect("copy to download temp path");
    let dest = dir.path().join(staged_binary_staging_name());

    extract_core_binary(&downloaded, &dest, true).expect("zip binary must extract");

    assert_eq!(
        std::fs::read(&dest).expect("read staged binary"),
        b"core binary"
    );
}

#[test]
fn raw_core_binary_stages_separately_from_running_executable() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let staged = staged_asset_path(dir.path(), staged_binary_name(), false);

    assert_eq!(staged, dir.path().join(staged_binary_staging_name()));
    assert_ne!(staged, dir.path().join(staged_binary_name()));
}

/// Linux releases publish a core archive, so a newer release without one for
/// this triple is a broken release and must stay an update-check failure.
#[cfg(target_os = "linux")]
#[test]
fn check_available_fails_on_linux_when_the_core_asset_is_missing() {
    let body = r#"{
        "tag_name": "v99.0.0",
        "body": "notes",
        "published_at": "2026-09-29T00:00:00Z",
        "assets": [
            {"name": "OpenHuman_99.0.0_aarch64.dmg", "browser_download_url": "https://example.invalid/dmg", "size": 1}
        ]
    }"#;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let result = runtime.block_on(async {
        let server = releases_mock(200, body).await;
        check_available_with_base_url(&server.uri()).await
    });
    let error = result.expect_err("a missing Linux core asset must fail the check");
    assert!(error.contains("no core asset was found"), "{error}");
}
