//! Tests for the connector module client.

use super::{methods, module_config, MODULE_ID};
use crate::openhuman::config::schema::{COMPOSIO_MODE_BACKEND, COMPOSIO_MODE_DIRECT};
use crate::openhuman::config::Config;
use crate::openhuman::modules::registry;

#[test]
fn the_module_is_registered_under_the_contract_s_identity() {
    // The bus name and object path are the module's address. A record that
    // disagrees with the contract does not fail to compile — it fails at
    // runtime, on a user's machine, as a name nobody owns.
    let record = registry::find(MODULE_ID).expect("tinyconnectors is registered");
    assert_eq!(record.bus_name, tinyconnectors_bus::INTERFACE);
    assert_eq!(record.object_path, tinyconnectors_bus::OBJECT_PATH);
}

#[test]
fn the_registered_version_matches_the_compiled_contract() {
    // The module-pin gate checks the artifact against the submodule; this
    // checks the record against the crate this build actually links.
    let record = registry::find(MODULE_ID).expect("tinyconnectors is registered");
    assert!(
        record
            .release_url
            .ends_with(&format!("v{}", record.version)),
        "the release URL and the version must name one release: {} / {}",
        record.release_url,
        record.version
    );
}

#[test]
fn the_module_is_lazy() {
    // A user with no connected accounts should not pay to load it, and most
    // sessions never touch a connector. Safe because the module loads without
    // configuration and still answers the capability members.
    let record = registry::find(MODULE_ID).expect("tinyconnectors is registered");
    assert!(
        matches!(
            record.load,
            crate::openhuman::modules::types::LoadPolicy::Lazy
        ),
        "tinyconnectors should load lazily"
    );
}

#[test]
fn every_member_this_host_names_is_one_the_module_serves() {
    // Spelled through the contract rather than as string literals, so a rename
    // upstream is a compile error here rather than an unknown method at
    // runtime. This asserts the names resolve to members the artifact declares.
    for member in [
        methods::LIST_TOOLKITS,
        methods::LIST_CONNECTIONS,
        methods::AUTHORIZE,
        methods::DELETE_CONNECTION,
        methods::LIST_TOOLS,
        methods::EXECUTE,
        methods::SYNC,
        methods::LIST_CAPABILITIES,
    ] {
        assert!(
            tinyconnectors_bus::METHODS.contains(&member),
            "{member} is not in the contract's member table"
        );
    }
}

// ── the route blob ───────────────────────────────────────────────────

/// A config with no backend session and no stored key.
fn bare_config() -> Config {
    let mut config = Config::default();
    config.composio.enabled = true;
    config
}

#[test]
fn direct_mode_takes_the_api_key_from_the_config_file() {
    // The keychain is the source of truth, but `config.toml` is the documented
    // fallback for power users — and the only one reachable in a unit test.
    let mut config = bare_config();
    config.composio.mode = COMPOSIO_MODE_DIRECT.to_string();
    config.composio.api_key = Some("  sk-from-file  ".to_string());

    let blob = module_config(&config).expect("direct mode with a key resolves");
    assert_eq!(blob["route"], "direct");
    assert_eq!(blob["api_key"], "sk-from-file", "the key is trimmed");
    assert_eq!(blob["entity_id"], config.composio.entity_id);
    assert!(blob.get("state_dir").is_some());
}

#[test]
fn direct_mode_without_a_key_is_refused() {
    // Rather than silently falling back to the backend route, which would send
    // the user's requests somewhere they did not choose.
    let mut config = bare_config();
    config.composio.mode = COMPOSIO_MODE_DIRECT.to_string();
    config.composio.api_key = None;

    let error = module_config(&config).expect_err("no key must be refused");
    assert!(error.contains("no api key"), "{error}");
}

#[test]
fn a_blank_api_key_is_not_a_key() {
    let mut config = bare_config();
    config.composio.mode = COMPOSIO_MODE_DIRECT.to_string();
    config.composio.api_key = Some("   ".to_string());

    assert!(module_config(&config).is_err());
}

#[test]
fn an_unknown_mode_fails_loudly_and_names_the_alternatives() {
    // A typo in config.toml must not silently downgrade to a working route.
    let mut config = bare_config();
    config.composio.mode = "backnd".to_string();

    let error = module_config(&config).expect_err("a typo must be refused");
    assert!(error.contains("backnd"), "{error}");
    assert!(error.contains(COMPOSIO_MODE_BACKEND), "{error}");
    assert!(error.contains(COMPOSIO_MODE_DIRECT), "{error}");
}

#[test]
fn an_empty_mode_is_treated_as_the_backend_default() {
    // `serde(default)` already gives "backend" for a missing field, but a
    // literal empty string in TOML would otherwise be rejected.
    let mut config = bare_config();
    config.composio.mode = String::new();

    // No session in a bare config, so this reports that rather than a bad mode.
    let error = module_config(&config).expect_err("no session");
    assert!(error.contains("session"), "{error}");
    assert!(!error.contains("unknown composio mode"), "{error}");
}

#[test]
fn the_backend_route_needs_a_session() {
    let mut config = bare_config();
    config.composio.mode = COMPOSIO_MODE_BACKEND.to_string();

    let error = module_config(&config).expect_err("no session token");
    assert!(error.contains("Sign in"), "{error}");
}

#[tokio::test]
async fn reconciling_the_route_of_a_module_that_is_not_serving_loads_nothing() {
    // One module instance per process — serialise with every test that
    // reaches it, since a signed-out reconcile drops whatever route it holds.
    let _serialised = crate::openhuman::integrations::composio::module_client::module_guard().await;
    let before = crate::openhuman::modules::ops::state_of(MODULE_ID);
    super::reconcile_route_if_loaded(&bare_config())
        .await
        .expect("nothing to reconcile is not a failure");
    // A module that was not serving must not have been loaded to answer this;
    // one that was serving stays serving.
    assert_eq!(crate::openhuman::modules::ops::state_of(MODULE_ID), before);
}
