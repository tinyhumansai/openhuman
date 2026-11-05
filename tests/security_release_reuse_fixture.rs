//! TinySecurity admitted from a pinned release, not a staged local file.
//!
//! The shipped app never sets `OPENHUMAN_MODULE_PATH`, so its TinySecurity is
//! a pinned release. The first load binds its host callbacks through the
//! callback factory; every later load must reuse that retained binding. This
//! target runs in its own process because module admission is process-wide.

use openhuman_core::{config::Config, modules::security::client};
use std::path::PathBuf;
use tinysecurity_bus::ProtectedPathRequest2;

#[tokio::test]
#[ignore = "loads the pinned TinySecurity release from a bundled-modules directory"]
async fn native_fixture_serves_later_calls_after_a_release_admission() {
    assert!(
        std::env::var_os("OPENHUMAN_MODULE_PATH").is_none(),
        "this fixture must not stage a local-file source"
    );
    assert!(
        std::env::var_os("OPENHUMAN_BUNDLED_MODULES").is_some(),
        "the fixture script provides the verified release as a bundled module"
    );
    let scratch = tempfile::tempdir().expect("scratch workspace");
    std::env::set_var("OPENHUMAN_WORKSPACE", scratch.path());
    std::env::set_var("OPENHUMAN_KEYRING_BACKEND", "file");
    let mut config = Config::default();
    config.modules.enabled = true;
    config.modules.allow_download = false;
    config.modules.install_dir = Some(scratch.path().join("modules").display().to_string());
    config.workspace_dir = scratch.path().to_path_buf();
    config.config_path = scratch.path().join("config.toml");
    config.action_dir = scratch.path().join("action");
    config.action_dir_override = Some(config.action_dir.clone());
    config.save().await.expect("save fixture config");
    openhuman_core::security::secret_storage::init_workspace(scratch.path());

    let first = client(&config)
        .await
        .expect("the pinned release should admit with its callback factory");
    first
        .policy_info()
        .await
        .expect("the admitted module should answer policy info");

    // Each later caller loads again; this is the path that refused every
    // non-local-file source.
    let second = client(&config)
        .await
        .expect("a later load must reuse the retained callback binding");
    let floor = second
        .protected_path2(ProtectedPathRequest2 {
            paths: vec![PathBuf::from("ordinary.txt")],
        })
        .await
        .expect("the protected floor should answer after admission");
    assert_eq!(floor.protected, vec![false]);

    // Redaction runs in the module on the same admitted binding.
    let redacted = second
        .redact(
            tinysecurity_bus::RedactMode::Text,
            "token=sk-ant-example-secret-value",
        )
        .await
        .expect("redaction should serve after admission");
    assert!(!redacted.text.contains("sk-ant-example-secret-value"));
}
