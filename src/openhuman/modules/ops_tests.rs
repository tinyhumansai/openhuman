//! Tests for module resolution and status reporting.
//!
//! Nothing here downloads. The paths that matter for correctness are the
//! refusals — disabled, unknown, unsupported, downloads-off — and each one is
//! reachable without touching the network, which is what keeps them in the unit
//! suite instead of behind an ignore.

use crate::openhuman::config::Config;
use crate::openhuman::modules::ops::{self, install_dir, list};
use crate::openhuman::modules::registry;
use crate::openhuman::modules::types::ModuleState;

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

#[test]
fn module_config_hands_the_module_the_hosts_cloud_embedding_defaults() {
    // `cloud_embedding_model` is what the module's engine falls back to when
    // the opted-in local model is unreachable, so it must be the host's
    // managed-cloud default, never the user's intended (usually local) model.
    // Sending `config.memory.embedding_model` here made the fallback ask the
    // managed embedder for `nomic-embed-text` (#5820).
    let mut config = offline_config();
    config.memory.embedding_model = "nomic-embed-text:latest".to_string();
    config.memory.embedding_dimensions = 768;

    let sent = ops::module_config(&config, crate::openhuman::modules::memory::MODULE_ID);

    assert_eq!(
        sent["cloud_embedding_model"],
        crate::openhuman::inference::embeddings::DEFAULT_CLOUD_EMBEDDING_MODEL
    );
    assert_eq!(
        sent["cloud_embedding_dimensions"],
        crate::openhuman::inference::embeddings::DEFAULT_CLOUD_EMBEDDING_DIMENSIONS
    );
    let supports = sent["models_supporting_dimensions"]
        .as_array()
        .expect("a list of model ids");
    assert!(
        supports
            .iter()
            .any(|model| model == "text-embedding-3-large"),
        "the dimension-aware family is named: {supports:?}"
    );
    // The user's own model still travels, just not as the cloud fallback.
    assert_eq!(sent["memory"]["embedding_model"], "nomic-embed-text:latest");
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
    let table = crate::openhuman::modules::resolution::table();
    table.reset_for_test("tinydocs");

    // Nothing to download from, nothing cached: the resolution settles at once,
    // so a bounded caller gets the terminal reason, never `StillLoading`.
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
    table.reset_for_test("tinydocs");

    match outcome {
        Err(ops::LoadError::Failed(reason)) => assert!(
            reason.contains("downloads are disabled")
                || reason.contains("not available for this platform"),
            "unhelpful message: {reason}"
        ),
        other => panic!("expected a terminal failure, got {other:?}"),
    }
    assert_eq!(state, ModuleState::Failed);
    assert_eq!(status.state, ModuleState::Failed);
    assert!(status.detail.is_some());
}

#[test]
fn each_artifact_of_a_version_has_its_own_cache_directory() {
    let record = registry::find("tinydocs").expect("tinydocs is a registry entry");
    let root = std::path::Path::new("/cache/modules");
    let dir = ops::artifact_dir(root, record, "macos-26-arm64").expect("a usable cache path");
    assert_eq!(
        dir,
        root.join("tinydocs")
            .join(record.version)
            .join("macos-26-arm64")
    );
    assert_ne!(Some(dir), ops::artifact_dir(root, record, "macos-15-arm64"));
}

#[test]
fn a_component_that_cannot_name_a_directory_yields_no_cache_path() {
    // The delete in `prune_stale_versions` is built from these components, so
    // a value that escapes its directory must produce no path at all rather
    // than one that resolves somewhere else.
    for bad in ["..", ".", "", "a/b", "a\\b", ".hidden", "a\0b"] {
        assert!(
            !ops::is_safe_path_component(bad),
            "{bad:?} must be refused as a directory name"
        );
    }
    for good in [
        "tinydocs",
        "0.1.15",
        "macos-26-arm64",
        "ubuntu-22.04-x86_64",
    ] {
        assert!(ops::is_safe_path_component(good), "{good:?} is a real name");
    }

    let record = registry::find("tinydocs").expect("tinydocs is a registry entry");
    let root = std::path::Path::new("/cache/modules");
    assert_eq!(ops::artifact_dir(root, record, ".."), None);
    assert_eq!(ops::artifact_dir(root, record, "a/b"), None);
    // Every shipped registry entry names a directory on every host it claims.
    for entry in registry::ALL {
        assert!(
            ops::is_safe_path_component(entry.id) && ops::is_safe_path_component(entry.version),
            "registry entry '{}' cannot name a cache directory",
            entry.id
        );
        for asset in entry.assets {
            assert!(
                ops::is_safe_path_component(asset.host_key),
                "'{}' host key '{}' cannot name a cache directory",
                entry.id,
                asset.host_key
            );
        }
    }
}

#[test]
fn pruning_keeps_the_pinned_version_and_anything_still_being_staged() {
    let record = registry::find("tinydocs").expect("tinydocs is a registry entry");
    let install = tempfile::tempdir().expect("temp install dir");
    let module_root = install.path().join(record.id);
    let pinned = module_root.join(record.version);
    let stale = module_root.join("0.0.1");
    let staging = module_root.join(".staging-abc123");
    for dir in [&pinned, &stale, &staging] {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("marker"), b"x").unwrap();
    }
    // A stray file beside the version directories is not a version.
    std::fs::write(module_root.join("notes.txt"), b"x").unwrap();

    ops::prune_stale_versions(install.path(), record);

    assert!(pinned.join("marker").is_file(), "the pinned version stays");
    assert!(
        staging.join("marker").is_file(),
        "an in-progress staging dir stays"
    );
    assert!(!stale.exists(), "an unpinned version is removed");
    assert!(module_root.join("notes.txt").is_file());

    // A module that was never cached has nothing to prune, and says nothing.
    ops::prune_stale_versions(&install.path().join("never"), record);
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
