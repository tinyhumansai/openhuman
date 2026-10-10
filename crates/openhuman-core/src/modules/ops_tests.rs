//! Tests for module resolution and status reporting.
//!
//! Nothing here downloads. The paths that matter for correctness are the
//! refusals — disabled, unknown, unsupported, downloads-off — and each one is
//! reachable without touching the network, which is what keeps them in the unit
//! suite instead of behind an ignore.

use crate::config::Config;
use crate::modules::ops::{self, install_dir, list};
use crate::modules::registry;
use crate::modules::types::ModuleState;

fn test_bundled_record() -> &'static crate::modules::types::ModuleRecord {
    use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset};

    let host_key = Box::leak(
        tinybus::module::platform::host_candidates()[0]
            .clone()
            .into_boxed_str(),
    );
    let assets = Box::leak(Box::new([PlatformAsset {
        host_key,
        archive: "test-bundled-module.zip",
        // SHA-256 of the empty archive staged by the test.
        sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
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
    }))
}

#[tokio::test]
async fn invalid_installer_bundle_is_reported_without_falling_back_to_the_cache() {
    let record = test_bundled_record();
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
    std::fs::write(bundle_dir.join(record.assets[0].archive), b"").unwrap();

    let runtime = crate::modules::host::runtime().await.unwrap();
    let error = ops::load_cached(
        runtime,
        record,
        user_cache.path(),
        serde_json::json!({}),
        false,
        Some(bundled.path()),
    )
    .unwrap_err();

    assert!(error.contains("installer bundle"), "{error}");
    assert!(error.contains("repairing the installation"), "{error}");
    assert!(!user_cache.path().join(record.id).exists());
}

#[tokio::test]
async fn absent_installer_bundle_uses_the_existing_cache_miss_path() {
    let record = test_bundled_record();
    let bundled = tempfile::tempdir().unwrap();
    let user_cache = tempfile::tempdir().unwrap();
    let runtime = crate::modules::host::runtime().await.unwrap();
    let error = ops::load_cached(
        runtime,
        record,
        user_cache.path(),
        serde_json::json!({}),
        false,
        Some(bundled.path()),
    )
    .unwrap_err();

    assert!(error.contains("downloads are disabled"), "{error}");
    assert!(!error.contains("installer bundle"), "{error}");
}

/// A config with modules on but downloads off, so nothing reaches the network.
fn offline_config() -> Config {
    let mut config = Config::default();
    config.modules.enabled = true;
    config.modules.allow_download = false;
    config
}

#[test]
fn the_default_config_enables_modules_and_downloads() {
    let config = Config::default();
    assert!(config.modules.enabled);
    assert!(config.modules.allow_download);
    assert!(config.modules.install_dir.is_none());
    assert!(config.modules.overrides.is_empty());
}

#[test]
fn list_reports_every_registry_entry() {
    let statuses = list(&offline_config());
    assert_eq!(statuses.len(), registry::ALL.len());
    assert!(statuses.iter().any(|status| status.id == "tinydocs"));
    for status in &statuses {
        assert!(!status.version.is_empty());
        assert!(!status.bus_name.is_empty());
    }
}

#[test]
fn module_statuses_remain_well_formed_after_other_tests_load_modules() {
    // Resolution is intentionally process-global. The full suite runs tests in
    // parallel, so another test may have loaded a module before this assertion
    // observes it. Verify the status contract without assuming test order.
    for status in list(&offline_config()) {
        if matches!(status.state, ModuleState::Unsupported | ModuleState::Failed) {
            assert!(
                status.detail.is_some(),
                "{} must explain its {:?} state",
                status.id,
                status.state
            );
        } else {
            assert!(
                status.detail.is_none(),
                "{} unexpectedly has detail for {:?}",
                status.id,
                status.state
            );
        }
    }
}

#[test]
fn disabling_modules_marks_everything_unsupported_with_a_reason() {
    let mut config = offline_config();
    config.modules.enabled = false;
    for status in list(&config) {
        assert_eq!(status.state, ModuleState::Unsupported);
        assert!(
            status
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("disabled")),
            "the reason should name the configuration, got {:?}",
            status.detail
        );
    }
}

#[tokio::test]
async fn a_disabled_host_refuses_before_starting_a_broker() {
    let mut config = offline_config();
    config.modules.enabled = false;
    let err = ops::ensure_loaded(&config, "tinydocs")
        .await
        .expect_err("modules are disabled");
    assert!(err.contains("disabled"), "unhelpful message: {err}");
}

#[tokio::test]
async fn an_unknown_module_is_refused_by_name() {
    let err = ops::ensure_loaded(&offline_config(), "not-a-module")
        .await
        .expect_err("unknown module");
    assert!(err.contains("not-a-module"), "unhelpful message: {err}");
}

#[test]
fn the_install_directory_is_namespaced_under_openhuman() {
    // Two arms where the first implies the second would make this vacuous, so
    // assert the components: artifacts land under an `openhuman` directory, in a
    // `modules` subdirectory, and never at the root of a shared cache.
    let dir = install_dir(&offline_config()).expect("an install directory is always resolvable");
    assert!(
        dir.ends_with("modules"),
        "install directory does not end in `modules`: {}",
        dir.display()
    );
    assert!(
        dir.parent()
            .and_then(|parent| parent.file_name())
            .is_some_and(|name| name == "openhuman"),
        "install directory is not namespaced under `openhuman`: {}",
        dir.display()
    );
}

#[test]
fn a_configured_install_directory_is_honoured() {
    let mut config = offline_config();
    config.modules.install_dir = Some("/tmp/openhuman-modules-test".to_string());
    assert_eq!(
        install_dir(&config).expect("configured"),
        std::path::PathBuf::from("/tmp/openhuman-modules-test")
    );
}

#[test]
fn errors_never_leak_a_path_or_a_url() {
    // Status details are rendered into a UI and pasted into bug reports.
    let mut config = offline_config();
    config.modules.enabled = false;
    for status in list(&config) {
        let detail = status.detail.unwrap_or_default();
        assert!(
            !detail.contains('/'),
            "a path leaked into a status: {detail}"
        );
        assert!(
            !detail.contains("http"),
            "a URL leaked into a status: {detail}"
        );
    }
}

#[tokio::test]
async fn a_bounded_wait_with_nothing_cached_and_downloads_off_fails_rather_than_loading() {
    // An isolated install directory: this machine's real cache may hold the
    // module, and a warm hit would turn the terminal refusal into a load.
    let install = tempfile::tempdir().expect("temp install dir");
    let mut config = offline_config();
    config.modules.install_dir = Some(install.path().display().to_string());

    // The resolution table is process-wide, and the module-backed document
    // tests populate this same slot when they are run with `--ignored`. A slot
    // left behind by one of those would be answered from cache before this
    // config is ever consulted, so clear it first and again at the end rather
    // than depending on which tests ran before this one.
    let table = tinybus::module::resolution::global();
    table.forget("tinydocs");

    // Nothing to download from, nothing cached: the resolution settles at once,
    // so a bounded caller gets the terminal reason, never `StillLoading`.
    // A previous test may already have loaded the native module into the
    // process-wide bus, however. Native modules cannot be unloaded safely, so
    // that legitimate process state is the one success outcome here.
    let outcome = ops::ensure_loaded_within(
        &config,
        "tinydocs",
        Some(std::time::Duration::from_secs(30)),
    )
    .await;
    // The outcome is remembered as a failure, and reported as one.
    let state = ops::state_of("tinydocs");
    let status = list(&config)
        .into_iter()
        .find(|status| status.id == "tinydocs")
        .expect("tinydocs is a registry entry");
    table.forget("tinydocs");

    match &outcome {
        Ok(()) => {
            assert_eq!(state, ModuleState::Ready);
            assert_eq!(status.state, ModuleState::Ready);
        }
        Err(ops::LoadError::Failed(reason)) => assert!(
            reason.contains("downloads are disabled")
                || reason.contains("not available for this platform"),
            "unhelpful message: {reason}"
        ),
        other => panic!("expected a terminal failure or an already-serving module, got {other:?}"),
    }
    if matches!(&outcome, Err(ops::LoadError::Failed(_))) {
        assert_eq!(state, ModuleState::Failed);
        assert_eq!(status.state, ModuleState::Failed);
        assert!(status.detail.is_some());
    }
}

#[test]
fn every_shipped_registry_entry_names_a_cache_directory() {
    // Every shipped registry entry names a directory on every host it claims.
    for entry in registry::ALL {
        assert!(
            tinybus::module::is_safe_path_component(entry.id)
                && tinybus::module::is_safe_path_component(entry.version),
            "registry entry '{}' cannot name a cache directory",
            entry.id
        );
        for asset in entry.assets {
            assert!(
                tinybus::module::is_safe_path_component(asset.host_key),
                "'{}' host key '{}' cannot name a cache directory",
                entry.id,
                asset.host_key
            );
        }
    }
}

#[test]
fn a_module_nobody_asked_for_is_available_not_loading() {
    assert_eq!(ops::state_of("never-asked"), ModuleState::Available);
}

#[test]
fn load_errors_render_for_callers_that_cannot_wait_again() {
    assert_eq!(
        ops::LoadError::Failed("refused".to_string()).into_message(),
        "refused"
    );
    let message = ops::LoadError::StillLoading.into_message();
    assert!(message.contains("still loading"), "{message}");
}

#[tokio::test]
async fn a_loader_error_carries_the_terminal_marker_exactly_once() {
    // `blocking` marks a loader error terminal, but tinybus' release-cache path
    // and `load_local` already say so; appending again doubled the sentence.
    // The failure policy keys on both phrases, so each must stay, once.
    let marker = crate::tools::status::MODULE_FAULT_MARKER;
    for loader_error in [
        "module 'tinydocs' could not be loaded: digest mismatch. This is terminal for the \
         running process; restart the app to try again",
        "module 'tinydocs' could not be loaded from the installer bundle: refused. Restart the \
         app after repairing the installation",
    ] {
        let owned = loader_error.to_string();
        let error = ops::blocking(move || Err(owned)).await.unwrap_err();
        assert_eq!(error.matches(marker).count(), 1, "{error}");
        assert_eq!(
            error.matches("restart the app to try again").count(),
            1,
            "{error}"
        );
    }
}

/// The Sentry payload of TAURI-RUST-117K: a load refused at admission, with the
/// terminal marker the loader adds.
const REFUSED_LOAD: &str = "module 'tinyconnectors' could not be loaded from the installer \
     bundle: module `windows-2022-x86_64` refused: module directory is writable by another \
     user. Restart the app after repairing the installation. This is terminal for the running \
     process; restart the app to try again";

#[cfg(feature = "crash-reporting")]
#[test]
fn a_failed_resolution_is_reported_once_with_the_module_id() {
    // The resolution runs once per process and caches its failure, so this
    // report is the one Sentry event per broken install. Every later caller's
    // re-report is demoted as `ModuleUnavailable`; this one must not be.
    let events = sentry::test::with_captured_events(|| {
        ops::report_resolution_failure(
            "tinyconnectors",
            "private-test-token /home/private-user/document.txt",
        );
    });
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(
        events[0].message.as_deref(),
        Some("loadable module operation failed")
    );
    let captured = format!("{:?}", events[0]);
    assert!(!captured.contains("private-test-token"));
    assert!(!captured.contains("/home/private-user"));
    let tags = &events[0].tags;
    assert_eq!(
        tags.get("reason_code").map(String::as_str),
        Some("resolution_failed")
    );
    assert_eq!(
        tags.get("version").map(String::as_str),
        Some(registry::find("tinyconnectors").unwrap().version)
    );
    assert_eq!(tags.get("domain").map(String::as_str), Some("modules"));
    assert_eq!(tags.get("operation").map(String::as_str), Some("resolve"));
    assert_eq!(
        tags.get("module").map(String::as_str),
        Some("tinyconnectors")
    );

    // The same reason re-reported by a caller is demoted.
    let repeats = sentry::test::with_captured_events(|| {
        crate::core::observability::report_error_or_expected(
            REFUSED_LOAD,
            "composio",
            "list_connections",
            &[],
        );
    });
    assert!(repeats.is_empty(), "{repeats:?}");
}

#[test]
fn bundled_dir_prefers_registered_then_env_then_exe_sibling() {
    let root = tempfile::tempdir().unwrap();
    let registered = root.path().join("registered");
    let from_env = root.path().join("env");
    let exe_dir = root.path().join("bin");
    std::fs::create_dir_all(&registered).unwrap();
    std::fs::create_dir_all(&from_env).unwrap();
    std::fs::create_dir_all(exe_dir.join("bundled-modules")).unwrap();

    assert_eq!(
        ops::resolve_bundled_dir(
            Some(registered.clone()),
            Some(from_env.clone()),
            Some(exe_dir.clone())
        ),
        Some(registered)
    );
    assert_eq!(
        ops::resolve_bundled_dir(None, Some(from_env.clone()), Some(exe_dir.clone())),
        Some(from_env)
    );
    assert_eq!(
        ops::resolve_bundled_dir(None, None, Some(exe_dir.clone())),
        Some(exe_dir.join("bundled-modules"))
    );
}

#[test]
fn bundled_dir_ignores_paths_that_do_not_exist() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(
        ops::resolve_bundled_dir(None, Some(root.path().join("missing")), None),
        None
    );
    assert_eq!(
        ops::resolve_bundled_dir(None, None, Some(root.path().into())),
        None
    );
}

#[test]
fn bundled_dir_skips_a_missing_candidate_for_a_valid_later_one() {
    let root = tempfile::tempdir().unwrap();
    let exe_dir = root.path().join("bin");
    std::fs::create_dir_all(exe_dir.join("bundled-modules")).unwrap();
    assert_eq!(
        ops::resolve_bundled_dir(
            Some(root.path().join("stale")),
            Some(root.path().join("typo")),
            Some(exe_dir.clone())
        ),
        Some(exe_dir.join("bundled-modules"))
    );
}

/// A module-bus startup failure is cached like any loader error, so it must
/// carry the terminal marker the observability classifier demotes on.
#[test]
fn a_marked_bus_startup_failure_is_classified_as_module_unavailable() {
    let marked = ops::mark_terminal("the module bus could not start: no runtime".to_string());
    assert!(marked.ends_with("restart the app to try again"), "{marked}");
    assert!(crate::core::observability::is_module_unavailable_message(
        &marked
    ));
    // Idempotent: an already-marked error is not annotated twice.
    assert_eq!(ops::mark_terminal(marked.clone()), marked);
}

#[tokio::test]
async fn disabled_modules_are_reported_then_demoted_at_product_boundaries() {
    let mut config = Config::default();
    config.modules.enabled = false;
    let reason = ops::ensure_loaded(&config, "tinyjuice").await.unwrap_err();
    assert!(crate::core::observability::is_module_unavailable_message(
        &reason
    ));
}
