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
#[cfg(feature = "crash-reporting")]
use tinybus::module::Reason;
use tinybus::module::{LoadError, Resolver};

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
        source: crate::modules::types::SourceSpec {
            submodule: "tinydocs",
            manifest: "Cargo.toml",
            package: "test-bundled-module",
            lib_name: "test_bundled_module",
            features: &[],
        },
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

    assert!(error.contains("installer bundle"), "{error}");
    assert!(error.contains("repairing the installation"), "{error}");
    assert!(!user_cache.path().join(record.id).exists());
}

#[tokio::test]
async fn absent_installer_bundle_uses_the_existing_cache_miss_path() {
    let record = test_bundled_record();
    let bundled = tempfile::tempdir().unwrap();
    let user_cache = tempfile::tempdir().unwrap();
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
    assert!(statuses.iter().any(|status| status.id == "tinymcp"));
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

#[cfg(feature = "documents")]
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
        Err(LoadError::Failed(reason)) => assert!(
            reason.contains("downloads are disabled")
                || reason.contains("not available for this platform"),
            "unhelpful message: {reason}"
        ),
        other => panic!("expected a terminal failure or an already-serving module, got {other:?}"),
    }
    if matches!(&outcome, Err(LoadError::Failed(_))) {
        assert_eq!(state, ModuleState::Failed);
        assert_eq!(status.state, ModuleState::Failed);
        assert!(status.detail.is_some());
    }
}

#[cfg(feature = "crash-reporting")]
#[test]
fn terminal_resolution_cached_failure_is_reported_when_a_client_becomes_available() {
    const MODULE: &str = "tinycomputer";
    let table = tinybus::module::resolution::global();
    table.forget(MODULE);

    // Model resolution finishing before core/Sentry startup, then publish its
    // terminal outcome into the same process-wide table read by ensure_loaded.
    let hub = std::sync::Arc::new(sentry::Hub::new_from_top(sentry::Hub::current()));
    hub.bind_client(None);
    sentry::Hub::run(hub, || {
        let record = registry::find(MODULE).unwrap();
        crate::modules::report::reporter().report(
            record.id,
            record.version,
            Reason::ResolutionFailed,
        );
    });
    let sender = table.mark_in_flight(MODULE);
    table.complete(
        MODULE,
        tinybus::module::resolution::Resolution::Failed(
            "private token /home/private-user/module.so".to_string(),
        ),
        sender,
    );

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let events = sentry::test::with_captured_events(|| {
        runtime.block_on(async {
            let config = Config::default();
            let (first, second) = tokio::join!(
                ops::ensure_loaded_within(&config, MODULE, None),
                ops::ensure_loaded_within(&config, MODULE, None),
            );
            for result in [first, second] {
                assert!(matches!(result, Err(LoadError::Failed(_))));
            }
            let repeated = ops::ensure_loaded_within(&config, MODULE, None)
                .await
                .expect_err("the same cached failure remains terminal");
            assert!(matches!(repeated, LoadError::Failed(_)));
        });
    });
    table.forget(MODULE);
    assert_eq!(events.len(), 1, "{events:?}");
    let captured = format!("{:?}", events[0]);
    assert!(!captured.contains("private token"));
    assert!(!captured.contains("/home/private-user"));
    assert_eq!(
        events[0].tags.get("reason_code").map(String::as_str),
        Some("resolution_failed")
    );
}

#[cfg(feature = "crash-reporting")]
#[test]
fn terminal_resolution_wait_timeout_is_not_reported_as_terminal() {
    const MODULE: &str = "tinyvoice";
    let table = tinybus::module::resolution::global();
    table.forget(MODULE);
    let _sender = table.mark_in_flight(MODULE);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let events = sentry::test::with_captured_events(|| {
        runtime.block_on(async {
            let result = ops::ensure_loaded_within(
                &Config::default(),
                MODULE,
                Some(std::time::Duration::ZERO),
            )
            .await;
            assert!(matches!(result, Err(LoadError::StillLoading)));
        });
    });
    table.forget(MODULE);
    assert!(events.is_empty(), "{events:?}");
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
        LoadError::Failed("refused".to_string()).into_message(),
        "refused"
    );
    let message = LoadError::StillLoading.into_message();
    assert!(message.contains("still loading"), "{message}");
}

#[test]
fn a_loader_error_carries_the_terminal_marker_exactly_once() {
    // The resolver marks a loader error terminal, but tinybus' release-cache path
    // and `load_local` already say so; appending again doubled the sentence.
    // The failure policy keys on both phrases, so each must stay, once.
    let marker = tinytools::status::MODULE_FAULT_MARKER;
    for loader_error in [
        "module 'tinydocs' could not be loaded: digest mismatch. This is terminal for the \
         running process; restart the app to try again",
        "module 'tinydocs' could not be loaded from the installer bundle: refused. Restart the \
         app after repairing the installation",
    ] {
        let error = ops::resolver(&Config::default()).mark_terminal(loader_error.to_string());
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
        let record = registry::find("tinyconnectors").unwrap();
        crate::modules::report::reporter().report(
            record.id,
            record.version,
            Reason::ResolutionFailed,
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
    let marked = ops::resolver(&Config::default())
        .mark_terminal("the module bus could not start: no runtime".to_string());
    assert!(marked.ends_with("restart the app to try again"), "{marked}");
    assert!(crate::core::observability::is_module_unavailable_message(
        &marked
    ));
    // Idempotent: an already-marked error is not annotated twice.
    assert_eq!(
        ops::resolver(&Config::default()).mark_terminal(marked.clone()),
        marked
    );
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

#[test]
fn status_reports_the_source_that_serves_a_module() {
    use crate::config::schema::ModuleSourceMode;
    use crate::modules::types::ModuleSource;

    let mut config = Config::default();
    let status = |config: &Config| {
        list(config)
            .into_iter()
            .find(|m| m.id == "tinyjuice")
            .expect("tinyjuice is registered")
    };
    if let Some(source) = status(&config).source {
        assert_eq!(source, ModuleSource::Release);
    }
    assert_eq!(status(&config).detail, None);

    // A linked request in a binary with no linked factory is served by the
    // release, and the status says why without naming a path.
    config.modules.source = ModuleSourceMode::Linked;
    let fallback = status(&config);
    assert_ne!(fallback.source, Some(ModuleSource::Linked));
    let detail = fallback.detail.expect("fallback is visible in detail");
    assert!(detail.contains("release"), "{detail}");
    assert!(!detail.contains('/'), "{detail}");

    // A path override is reported as a local file.
    let mut config = Config::default();
    config
        .modules
        .overrides
        .push(crate::config::schema::ModuleOverride::with_path(
            "tinyjuice",
            "/dev/libtinyjuice_module.so",
        ));
    let local = status(&config);
    assert_ne!(local.source, Some(ModuleSource::Release));
    if let Some(source) = local.source {
        assert_eq!(source, ModuleSource::LocalFile);
    }
}

#[tokio::test]
// The tinychannels fixture compiles only with `channels` (channels/mod.rs).
#[cfg(all(feature = "e2e-test-support", feature = "channels"))]
async fn a_registered_linked_factory_serves_the_module_and_status_says_so() {
    use crate::config::schema::ModuleSourceMode;
    use crate::modules::types::ModuleSource;

    let _storage = crate::channels::module_fixture::install_memory_storage();
    crate::modules::linked::register_linked("tinychannels", tinychannels_module::linked_module);
    let mut config = Config::default();
    config.modules.source = ModuleSourceMode::Linked;
    ops::ensure_loaded(&config, "tinychannels")
        .await
        .expect("the linked module is admitted through the seam");

    let status = list(&config)
        .into_iter()
        .find(|m| m.id == "tinychannels")
        .expect("tinychannels is registered");
    assert_eq!(status.state, ModuleState::Ready);
    assert_eq!(status.source, Some(ModuleSource::Linked));
    assert_eq!(status.detail, None);
}

#[tokio::test]
async fn attaching_a_linked_module_without_a_factory_fails_with_a_static_reason() {
    let runtime = crate::modules::runtime::runtime()
        .await
        .expect("module host");
    let error = Resolver::load_linked(
        runtime,
        &registry::find("tinychannels").unwrap().release(),
        None,
        serde_json::json!({}),
    )
    .expect_err("no factory is registered for this id");
    assert!(error.contains("no linked factory"), "{error}");
    assert!(!error.contains('/'), "{error}");
}

#[tokio::test]
async fn a_failing_linked_factory_is_reported_without_its_error_detail() {
    fn failing() -> tinybus::Result<tinybus::module::LinkedModule> {
        Err(tinybus::Error::failed("secret-token /home/user/path"))
    }
    let runtime = crate::modules::runtime::runtime()
        .await
        .expect("module host");
    let error = Resolver::load_linked(
        runtime,
        &registry::find("tinychannels").unwrap().release(),
        Some(failing),
        serde_json::json!({}),
    )
    .expect_err("the factory fails");
    assert!(
        error.contains("linked factory could not be started"),
        "{error}"
    );
    assert!(!error.contains("secret-token"), "{error}");
    assert!(!error.contains("/home"), "{error}");
}

#[test]
fn a_debug_build_warns_when_a_local_build_is_served_ahead_of_its_pin() {
    use crate::modules::source::pin_ahead_warning;
    use crate::modules::types::ModuleSource;

    for served in [ModuleSource::Linked, ModuleSource::Source] {
        let line = pin_ahead_warning("tinydocs", served, Some("tinysearch, tinydocs"), true)
            .expect("an ahead id served locally warns");
        assert!(line.contains("id=tinydocs"));
        assert!(line.contains("gitlink ahead of released pin"));
        assert!(line.contains("release-and-repin"));
    }
}

#[test]
fn the_ahead_warning_is_silent_for_released_modules_other_ids_and_release_builds() {
    use crate::modules::source::pin_ahead_warning;
    use crate::modules::types::ModuleSource;

    // Released and explicit-file modules are not a locally compiled pin.
    for served in [ModuleSource::Release, ModuleSource::LocalFile] {
        assert!(pin_ahead_warning("tinydocs", served, Some("tinydocs"), true).is_none());
    }
    // Not in the list, an id that merely contains the name, no list at all.
    assert!(pin_ahead_warning("tinydocs", ModuleSource::Source, Some("tinydocs2"), true).is_none());
    assert!(pin_ahead_warning("tinydocs", ModuleSource::Source, Some(""), true).is_none());
    assert!(pin_ahead_warning("tinydocs", ModuleSource::Source, None, true).is_none());
    // A release build never warns.
    assert!(pin_ahead_warning("tinydocs", ModuleSource::Source, Some("tinydocs"), false).is_none());
}
