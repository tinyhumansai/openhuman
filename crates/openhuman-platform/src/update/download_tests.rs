use super::*;
use crate::update::test_support::{RecordingReporter, SOURCE};
use crate::update::{download_and_stage, platform_triple};

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ---------------------------------------------------------------------------
// #315: the staging path trusted whatever body the download URL returned. The
// release publishes a `.sha256` beside each archive and the release metadata
// declares the asset size; neither was checked, and the body was buffered
// whole. These drive `download_and_stage` against a local mock.
// ---------------------------------------------------------------------------

fn sha256(body: &[u8]) -> [u8; 32] {
    Sha256::digest(body).into()
}

/// Mount `body` at `/<asset>` and, when given, `sidecar` at `/<asset>.sha256`.
async fn release_mock(asset: &str, body: Vec<u8>, sidecar: Option<String>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/{asset}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .mount(&server)
        .await;
    let sidecar_response = match sidecar {
        Some(text) => ResponseTemplate::new(200).set_body_string(text),
        None => ResponseTemplate::new(404),
    };
    Mock::given(method("GET"))
        .and(path(format!("/{asset}.sha256")))
        .respond_with(sidecar_response)
        .mount(&server)
        .await;
    server
}

/// A legacy raw-binary asset name, which stages on every platform without an
/// archive.
fn raw_asset() -> String {
    format!("openhuman-core-{}", platform_triple())
}

fn assert_nothing_staged(dir: &std::path::Path) {
    let leftovers: Vec<_> = std::fs::read_dir(dir)
        .expect("read staging dir")
        .map(|entry| entry.expect("dir entry").file_name())
        .collect();
    assert!(
        leftovers.is_empty(),
        "nothing may be staged or left behind on refusal, found: {leftovers:?}"
    );
}

#[tokio::test]
async fn staging_refuses_a_tampered_archive_and_stages_nothing() {
    let asset = raw_asset();
    let genuine = b"genuine core binary".to_vec();
    let tampered = b"tampered core binary".to_vec();
    let sidecar = format!("{}\n", hex::encode(sha256(&genuine)));
    let server = release_mock(&asset, tampered.clone(), Some(sidecar)).await;
    let dir = tempfile::TempDir::new().expect("temp dir");

    let result = download_and_stage(
        &SOURCE,
        &format!("{}/{asset}", server.uri()),
        &asset,
        Some(dir.path().to_path_buf()),
        Some(tampered.len() as u64),
        &RecordingReporter::default(),
    )
    .await;

    let err = result.expect_err("a body whose sha256 does not match the sidecar must be refused");
    assert!(
        err.contains("checksum mismatch"),
        "error must name the checksum, got: {err}"
    );
    assert_nothing_staged(dir.path());
}

#[tokio::test]
async fn staging_refuses_a_body_larger_than_the_declared_size() {
    // The sidecar matches the oversized body, so only the size cap stops it.
    let asset = raw_asset();
    let body = vec![b'x'; 4096];
    let sidecar = hex::encode(sha256(&body));
    let server = release_mock(&asset, body, Some(sidecar)).await;
    let dir = tempfile::TempDir::new().expect("temp dir");

    let result = download_and_stage(
        &SOURCE,
        &format!("{}/{asset}", server.uri()),
        &asset,
        Some(dir.path().to_path_buf()),
        Some(16),
        &RecordingReporter::default(),
    )
    .await;

    let err = result.expect_err("a body larger than the declared size must be refused");
    assert!(
        err.contains("exceeds the declared size of 16 bytes"),
        "error must name the cap, got: {err}"
    );
    assert_nothing_staged(dir.path());
}

#[tokio::test]
async fn staging_refuses_an_update_without_a_checksum_sidecar() {
    let asset = raw_asset();
    let server = MockServer::start().await;
    // Fail closed before the archive is even requested.
    Mock::given(method("GET"))
        .and(path(format!("/{asset}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"core".to_vec()))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{asset}.sha256")))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let dir = tempfile::TempDir::new().expect("temp dir");

    let result = download_and_stage(
        &SOURCE,
        &format!("{}/{asset}", server.uri()),
        &asset,
        Some(dir.path().to_path_buf()),
        Some(4),
        &RecordingReporter::default(),
    )
    .await;

    let err = result.expect_err("an update without a checksum sidecar must be refused");
    assert!(
        err.contains("checksum sidecar unavailable"),
        "error must name the missing sidecar, got: {err}"
    );
    assert_nothing_staged(dir.path());
}

#[tokio::test]
async fn staging_refuses_a_malformed_checksum_sidecar() {
    let asset = raw_asset();
    let body = b"core".to_vec();
    let server = release_mock(&asset, body, Some("not a digest".to_string())).await;
    let dir = tempfile::TempDir::new().expect("temp dir");

    let result = download_and_stage(
        &SOURCE,
        &format!("{}/{asset}", server.uri()),
        &asset,
        Some(dir.path().to_path_buf()),
        None,
        &RecordingReporter::default(),
    )
    .await;

    assert!(result.is_err(), "a malformed sidecar must be refused");
    assert_nothing_staged(dir.path());
}

/// Build an in-memory `.tar.gz` holding one `openhuman-core` entry.
#[cfg(unix)]
fn tar_gz_with_core(contents: &[u8]) -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut builder = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_size(contents.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    builder
        .append_data(&mut header, "openhuman-core", contents)
        .expect("append entry");
    builder
        .into_inner()
        .expect("finish tar")
        .finish()
        .expect("finish gz")
}

#[cfg(unix)]
#[tokio::test]
async fn staging_stages_a_verified_archive() {
    let asset = format!("openhuman-core-99.0.0-{}.tar.gz", platform_triple());
    let archive = tar_gz_with_core(b"#!/bin/sh\nexit 0\n");
    // The `sha256sum` form, which carries the file name after the digest.
    let sidecar = format!("{}  {asset}\n", hex::encode(sha256(&archive)));
    let size = archive.len() as u64;
    let server = release_mock(&asset, archive, Some(sidecar)).await;
    let dir = tempfile::TempDir::new().expect("temp dir");

    let applied = download_and_stage(
        &SOURCE,
        &format!("{}/{asset}", server.uri()),
        &asset,
        Some(dir.path().to_path_buf()),
        Some(size),
        &RecordingReporter::default(),
    )
    .await
    .expect("a verified archive must stage");

    assert_eq!(
        std::fs::read(&applied.staged_path).expect("read staged binary"),
        b"#!/bin/sh\nexit 0\n",
        "the staged file must be the inner binary"
    );
    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .expect("read staging dir")
        .map(|entry| entry.expect("dir entry").path())
        .collect();
    assert_eq!(
        entries,
        vec![applied.staged_path.clone()],
        "the downloaded archive must not be left beside the staged binary"
    );
}

#[test]
fn checksum_url_appends_to_the_path() {
    assert_eq!(
        checksum_url("https://github.com/o/r/releases/download/v1/core.tar.gz").unwrap(),
        "https://github.com/o/r/releases/download/v1/core.tar.gz.sha256"
    );
    assert_eq!(
        checksum_url("https://example.com/core.tar.gz?token=abc").unwrap(),
        "https://example.com/core.tar.gz.sha256?token=abc",
        "a query string must stay after the suffixed path"
    );
}

#[test]
fn parse_checksum_reads_both_published_forms() {
    let digest = sha256(b"core");
    let hex_digest = hex::encode(digest);

    assert_eq!(parse_checksum(&format!("{hex_digest}\n")).unwrap(), digest);
    assert_eq!(
        parse_checksum(&format!("{hex_digest}  core.tar.gz\n")).unwrap(),
        digest
    );
    assert!(parse_checksum("").is_err());
    assert!(parse_checksum("zz").is_err());
    assert!(
        parse_checksum(&hex_digest[..62]).is_err(),
        "a short digest must not parse"
    );
}

#[test]
fn verify_refuses_a_truncated_body() {
    let digest = sha256(b"core");

    let err = verify(3, &digest, 4, Some(4), &digest).expect_err("3 of 4 bytes is truncated");
    assert!(err.contains("declared 4"), "{err}");
    assert!(verify(4, &digest, 4, Some(4), &digest).is_ok());
    assert!(
        verify(5, &digest, 4, None, &digest).is_err(),
        "the cap applies without a declared size too"
    );
}
