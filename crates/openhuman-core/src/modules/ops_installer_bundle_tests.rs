//! Installer-bundle regression tests for `ops`.
//!
//! Carved out of `ops_tests.rs` to keep that file under the 750-line layout
//! limit (`scripts/ci/check-openhuman-rust-layout.mjs`). The sibling tests in
//! `ops_tests.rs` still cover the matching-digest "pin matches, extract
//! fails" path (`an_unreadable_installer_bundle_is_reported_without_falling_back_to_the_cache`);
//! this file holds the distinct "pin mismatches on disk" path.

use tinybus::module::Resolver;

/// Variant of `test_bundled_record` whose asset sha256 pin is a dummy value
/// no real archive will ever hash to. Used to reproduce the exact production
/// condition behind TAURI-RUST-11MZ / issue #8500: a bundled archive whose
/// bytes do not match the compiled-in registry pin (pin drift between a
/// staged bundle and the binary that reads it). `test_bundled_record` in
/// `ops_tests.rs` pins the empty-input digest and runs the "pin matches,
/// extract fails" path; this one runs the "pin mismatches on disk" path,
/// which is a distinct branch of the admission flow.
fn test_bundled_record_with_mismatched_pin() -> &'static crate::modules::types::ModuleRecord {
    use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset};

    let host_key = Box::leak(
        tinybus::module::platform::host_candidates()[0]
            .clone()
            .into_boxed_str(),
    );
    let assets = Box::leak(Box::new([PlatformAsset {
        host_key,
        archive: "test-bundled-module.zip",
        // Deliberately bogus pin: no real archive will ever hash to this.
        sha256: "feedfacefeedfacefeedfacefeedfacefeedfacefeedfacefeedfacefeedface",
        library_sha256: None,
    }]));
    Box::leak(Box::new(ModuleRecord {
        id: "test-bundled-module",
        description: "test module",
        bus_name: "test.BundledModule",
        object_path: "/test/BundledModule",
        version: "0.0.0",
        release_url: "https://github.com/tinyhumansai/tinydocs/releases/tag/v0.1.16",
        assets,
        load: LoadPolicy::Lazy,
        source: crate::modules::types::SourceSpec {
            submodule: "tinydocs",
            manifest: "Cargo.toml",
            package: "test-bundled-module",
            lib_name: "test_bundled_module",
            features: &[],
        },
    }))
}

/// Regression for TAURI-RUST-11MZ / issue #8500 — the digest-rejection
/// branch, as seen from the host.
///
/// The sibling test in `ops_tests.rs` runs the "pin matches on disk, extract
/// fails" path. This one runs the distinct "pin mismatches on disk" path: an
/// installer-bundle archive is present but its bytes do not hash to the
/// compiled-in pin, so tinybus's loader refuses it. Downloads are hard-
/// disabled in the installer-bundle branch, so the admission path has
/// nowhere else to fall through to.
///
/// Per AGENTS.md ("Module behavior is tested in the owning `vendor/tiny*`
/// repository"), the exact wording of the digest-rejection error is
/// tinybus's to pin; this test covers only the host-level contract:
///
/// * the error is surfaced to the caller scoped as an installer-bundle
///   failure (so Sentry groups it with the user-facing message);
/// * the user cache is not populated — a refused bundled archive must
///   never silently fall back to the download path.
#[tokio::test]
async fn installer_bundle_sha_mismatch_is_reported_and_does_not_populate_the_cache() {
    let record = test_bundled_record_with_mismatched_pin();
    let bundled = tempfile::tempdir().unwrap();
    let user_cache = tempfile::tempdir().unwrap();
    let bundle_dir = tinybus::module::artifact_dir(
        bundled.path(),
        record.id,
        record.version,
        record.assets[0].host_key,
    )
    .unwrap();
    std::fs::create_dir_all(&bundle_dir).unwrap();
    // Any non-empty bytes that do not hash to the deliberately-bogus pin.
    std::fs::write(
        bundle_dir.join(record.assets[0].archive),
        b"PK\x03\x04not-the-pinned-archive-bytes-at-all",
    )
    .unwrap();

    let runtime = crate::modules::runtime::runtime().await.unwrap();
    let error = Resolver::load_cached(
        runtime,
        &record.release(),
        user_cache.path(),
        &serde_json::json!({}),
        false,
        Some(bundled.path()),
    )
    .unwrap_err();

    // Host-level scope marker: the error must identify this as an
    // installer-bundle failure (not a generic download-disabled miss)
    // so the Sentry rule that files TAURI-RUST-11MZ keeps grouping it.
    assert!(error.contains("installer bundle"), "{error}");
    // The load-bearing host invariant: a refused bundled archive must
    // not touch the download cache. If this ever regresses, a pin-
    // drifted installer silently falls back to the network path.
    assert!(
        !user_cache.path().join(record.id).exists(),
        "a refused bundled archive must not touch the download cache"
    );
}
