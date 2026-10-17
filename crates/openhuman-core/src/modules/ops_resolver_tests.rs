//! Host-level regression tests for the resolver adapter: a faulted module is
//! never retried in the process, and `modules.list` keeps its wire shape.

use crate::config::Config;
use crate::modules::ops::{self, list};
use crate::modules::registry;
use crate::modules::types::ModuleState;
use tinybus::module::LoadError;

/// A config with modules on but downloads off, so nothing reaches the network.
fn offline_config() -> Config {
    let mut config = Config::default();
    config.modules.enabled = true;
    config.modules.allow_download = false;
    config
}

/// A faulted module stays faulted until the process restarts: tinybus never
/// unloads a library, so a retry would pay a download and a `dlopen` to reach
/// the same error. The cached outcome wins over whatever configuration the
/// later caller holds.
#[tokio::test]
async fn a_faulted_module_is_not_retried_in_process() {
    const ISOLATED: &str = "OPENHUMAN_TEST_ISOLATED_FAULTED_MODULE_RESOLVER";
    if std::env::var_os(ISOLATED).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "modules::ops::resolver_tests::a_faulted_module_is_not_retried_in_process",
                "--test-threads=1",
            ])
            .env(ISOLATED, "1")
            .output()
            .expect("run the resolver test in a fresh process");
        assert!(
            output.status.success(),
            "isolated resolver test failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    use crate::config::schema::ModuleOverride;
    const MODULE: &str = "tinysearch";
    let table = tinybus::module::resolution::global();
    table.forget(MODULE);

    let missing = tempfile::tempdir().unwrap().path().join("absent-module.so");
    let mut faulting = Config::default();
    faulting.modules.enabled = true;
    faulting.modules.overrides.push(ModuleOverride::with_path(
        MODULE,
        missing.to_string_lossy().into_owned(),
    ));
    let first = ops::ensure_loaded_within(&faulting, MODULE, None)
        .await
        .unwrap_err();
    let LoadError::Failed(first) = first else {
        panic!("a missing local artifact is a terminal failure: {first:?}");
    };
    assert!(first.contains("local artifact"), "{first}");
    assert!(first.contains("restart the app to try again"), "{first}");
    assert_eq!(ops::state_of(MODULE), ModuleState::Failed);

    // A later caller whose configuration would take a different rung (downloads
    // off, no override) still gets the cached outcome, not a second attempt.
    let mut offline = Config::default();
    offline.modules.enabled = true;
    offline.modules.allow_download = false;
    for _ in 0..3 {
        let again = ops::ensure_loaded_within(&offline, MODULE, None)
            .await
            .unwrap_err();
        assert_eq!(again, LoadError::Failed(first.clone()));
    }
    let status = list(&offline).into_iter().find(|m| m.id == MODULE).unwrap();
    assert_eq!(status.state, ModuleState::Failed);
    assert_eq!(status.detail.as_deref(), Some(first.as_str()));
    table.forget(MODULE);
}

/// `modules.list` is a wire contract: the field names and the snake_case state
/// and source spellings are what clients parse.
#[test]
fn modules_list_output_keeps_its_wire_shape() {
    let config = offline_config();
    let statuses = list(&config);
    assert_eq!(statuses.len(), registry::ALL.len());
    for (status, record) in statuses.iter().zip(registry::ALL) {
        assert_eq!(status.id, record.id);
        assert_eq!(status.description, record.description);
        assert_eq!(status.version, record.version);
        assert_eq!(status.bus_name, record.bus_name);
        let wire = serde_json::to_value(status).unwrap();
        let object = wire.as_object().unwrap();
        let allowed = [
            "id",
            "description",
            "version",
            "bus_name",
            "state",
            "source",
            "detail",
        ];
        for key in object.keys() {
            assert!(allowed.contains(&key.as_str()), "unexpected field {key}");
        }
        for key in ["id", "description", "version", "bus_name", "state"] {
            assert!(object.contains_key(key), "missing field {key}");
        }
        let state = object["state"].as_str().unwrap();
        assert!(
            [
                "available",
                "loading",
                "ready",
                "failed",
                "unsupported",
                "disabled"
            ]
            .contains(&state),
            "{state}"
        );
        if let Some(source) = object.get("source") {
            assert!(
                ["release", "linked", "source", "local_file"].contains(&source.as_str().unwrap()),
                "{source}"
            );
        }
    }
    let mut disabled = offline_config();
    disabled.modules.enabled = false;
    let wire = serde_json::to_value(&list(&disabled)[0]).unwrap();
    assert_eq!(wire["state"], "unsupported");
    assert_eq!(wire["detail"], "modules are disabled in configuration");
    assert!(wire.get("source").is_none());
}
