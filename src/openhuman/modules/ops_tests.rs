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
