//! Tests for the connector module client.

use super::{methods, module_config, module_config_with_proxy, MODULE_ID};
use crate::config::schema::{COMPOSIO_MODE_BACKEND, COMPOSIO_MODE_DIRECT};
use crate::config::Config;
use crate::modules::registry;

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
        matches!(record.load, crate::modules::types::LoadPolicy::Lazy),
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
    Config::default()
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

/// Removes the direct-base overrides for a test's duration and restores them.
/// The route blob's destination (and so whether a proxy applies to it) follows
/// them, and other tests point them at a loopback mock while they hold
/// `TEST_ENV_LOCK`.
struct DirectBaseScrub(Vec<(&'static str, Option<std::ffi::OsString>)>);

impl DirectBaseScrub {
    fn new() -> Self {
        let vars = [
            "OPENHUMAN_COMPOSIO_DIRECT_BASE_V2",
            "OPENHUMAN_COMPOSIO_DIRECT_BASE_V3",
        ];
        Self(
            vars.into_iter()
                .map(|name| {
                    let old = std::env::var_os(name);
                    std::env::remove_var(name);
                    (name, old)
                })
                .collect(),
        )
    }
}

impl Drop for DirectBaseScrub {
    fn drop(&mut self) {
        for (name, old) in self.0.drain(..) {
            if let Some(value) = old {
                std::env::set_var(name, value);
            }
        }
    }
}

#[test]
fn direct_mode_hands_the_module_the_hosts_proxy_policy() {
    // Part of the route description, so a proxy change reconfigures the module.
    use crate::config::schema::{ProxyConfig, ProxyScope};

    // The destination follows the direct-base env overrides; keep any other
    // test's loopback override out of this one.
    let _env = crate::config::TEST_ENV_LOCK.blocking_lock();
    let _scrub = DirectBaseScrub::new();

    let mut config = bare_config();
    config.composio.mode = COMPOSIO_MODE_DIRECT.to_string();
    config.composio.api_key = Some("sk-from-file".to_string());

    // The policy is passed in rather than installed as the process-global
    // runtime proxy: every config load republishes that global, so installing
    // it here raced any concurrent test that loads a config.
    let proxied = module_config_with_proxy(
        &config,
        &ProxyConfig {
            enabled: true,
            https_proxy: Some("http://127.0.0.1:3128".into()),
            no_proxy: vec!["localhost".into()],
            // Scoped to Composio so no other test's loopback request is proxied.
            scope: ProxyScope::Services,
            services: vec!["tool.composio".into()],
            ..ProxyConfig::default()
        },
    )
    .expect("resolves");
    let plain = module_config_with_proxy(&config, &ProxyConfig::default()).expect("resolves");

    assert_eq!(proxied["transport"]["proxy_url"], "http://127.0.0.1:3128");
    assert_eq!(proxied["transport"]["no_proxy"][0], "localhost");
    if !cfg!(target_os = "windows") {
        assert!(
            plain.get("transport").is_none(),
            "the default policy is not sent"
        );
    }
    assert_ne!(
        super::fingerprint(&proxied),
        super::fingerprint(&plain),
        "a proxy change must change the route fingerprint"
    );
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
    crate::integrations::composio::test_backend_route::register();
    let mut config = bare_config();
    config.composio.mode = String::new();

    // No session in a bare config, so this reports that rather than a bad mode.
    let error = module_config(&config).expect_err("no session");
    assert!(error.contains("session"), "{error}");
    assert!(!error.contains("unknown composio mode"), "{error}");
}

#[test]
fn the_backend_route_is_unavailable_without_a_registered_target() {
    let mut config = bare_config();
    config.composio.mode = COMPOSIO_MODE_BACKEND.to_string();

    let error = module_config(&config).expect_err("no proxy target is registered");
    assert!(
        error.starts_with(crate::core::observability::BACKEND_UNAVAILABLE_PREFIX),
        "{error}"
    );
}

#[test]
fn the_backend_route_needs_a_session() {
    crate::integrations::composio::test_backend_route::register();
    let mut config = bare_config();
    config.composio.mode = COMPOSIO_MODE_BACKEND.to_string();

    let error = module_config(&config).expect_err("no session token");
    assert!(error.contains("Sign in"), "{error}");
}

#[tokio::test]
async fn reconciling_the_route_of_a_module_that_is_not_serving_loads_nothing() {
    // One module instance per process — serialise with every test that
    // reaches it, since a signed-out reconcile drops whatever route it holds.
    let _serialised = crate::integrations::composio::module_client::module_guard().await;
    let before = crate::modules::ops::state_of(MODULE_ID);
    super::reconcile_route_if_loaded(&bare_config())
        .await
        .expect("nothing to reconcile is not a failure");
    // A module that was not serving must not have been loaded to answer this;
    // one that was serving stays serving.
    assert_eq!(crate::modules::ops::state_of(MODULE_ID), before);
}

#[test]
fn unclassified_connector_faults_do_not_expose_remote_content() {
    let error = tinybus::Error::MethodFailed {
        name: "private-provider".into(),
        message: "secret-token /home/private-user/document.txt".into(),
    };
    let message = super::bus_failure(methods::EXECUTE, &error);
    assert!(message.contains("MODULE_CALL_REPORTED: module execution failed"));
    assert!(!message.contains("secret-token"));
    assert!(!message.contains("/home/private-user"));
}

#[cfg(feature = "crash-reporting")]
#[test]
fn classified_provider_errors_remain_product_output_and_emit_one_sanitized_event() {
    let provider_message =
        "[composio:error:auth] private-test-token /home/private-user/document.txt";
    let error = tinybus::Error::MethodFailed {
        name: "ai.tinyhumans.tinybus.Error.Failed".into(),
        message: provider_message.into(),
    };
    let events = sentry::test::with_captured_events(|| {
        let message = super::bus_failure(methods::EXECUTE, &error);
        assert_eq!(message, provider_message);
        crate::core::observability::report_error_or_expected(&message, "rpc", "invoke_method", &[]);
    });
    assert_eq!(events.len(), 1);
    let captured = format!("{:?}", events[0]);
    assert!(!captured.contains("private-test-token"));
    assert!(!captured.contains("/home/private-user"));
    assert_eq!(
        events[0].tags.get("module").map(String::as_str),
        Some(MODULE_ID)
    );
    assert_eq!(
        events[0].tags.get("reason_code").map(String::as_str),
        Some("module_fault")
    );
}

#[test]
fn direct_auth_refusals_preserve_the_provider_status_and_prevent_duplicate_reports() {
    let error =
        tinybus::Error::failed("Composio v3 connected_accounts failed: HTTP 401: Invalid API key");
    let message = super::bus_failure(methods::LIST_CONNECTIONS_DIRECT, &error);
    assert!(message.contains("HTTP 401: Invalid API key"));
    assert!(crate::core::observability::is_module_unavailable_message(
        &message
    ));
}

#[test]
fn a_host_pinned_credential_outranks_the_mode_and_the_store() {
    let mut config = bare_config();
    config.composio.mode = COMPOSIO_MODE_BACKEND.into();
    config.composio.pin_host_credential(
        crate::config::ComposioHostCredential::direct("ck_pinned")
            .entity_id("tenant-7")
            .base_urls("http://127.0.0.1:1/v2", "http://127.0.0.1:1/v3"),
    );
    let route = module_config(&config).expect("pinned credential names a route");
    assert_eq!(route["route"], "direct");
    assert_eq!(route["api_key"], "ck_pinned");
    assert_eq!(route["entity_id"], "tenant-7");
    assert_eq!(route["base_url"], "http://127.0.0.1:1/v3");
}

#[test]
fn the_offered_toolkit_lists_reach_the_module_in_its_load_blob() {
    let mut config = bare_config();
    config.composio.mode = COMPOSIO_MODE_DIRECT.into();
    config.composio.api_key = Some("ck_test".into());
    config.composio.offered_toolkits_allow = vec!["gmail".into(), "slack".into()];
    config.composio.offered_toolkits_deny = vec!["slack".into()];

    let blob = module_config(&config).expect("direct mode with a key resolves");
    assert_eq!(
        blob["offered_toolkits"],
        serde_json::json!({ "allow": ["gmail", "slack"], "deny": ["slack"] })
    );
    // And the module parses it as its policy: deny wins over allow.
    let policy: tinyconnectors_bus::vocabulary::OfferedToolkits =
        serde_json::from_value(blob["offered_toolkits"].clone()).expect("the module's own shape");
    assert!(policy.permits("gmail"));
    assert!(!policy.permits("slack"));
}

#[test]
fn an_unconfigured_route_still_carries_the_offered_toolkit_policy() {
    let mut config = bare_config();
    config.composio.offered_toolkits_deny = vec!["twitter".into()];
    // No session and no key: no route can be named, but the load blob the
    // module falls back to must not drop the operator's policy.
    assert!(module_config(&config).is_err());
    let fallback = super::ops::module_config_for_test(&config, MODULE_ID);
    assert_eq!(
        fallback["offered_toolkits"]["deny"],
        serde_json::json!(["twitter"])
    );
}

#[test]
fn offered_toolkits_default_to_unrestricted_and_parse_from_toml() {
    let defaults = Config::default();
    assert!(defaults.composio.offered_toolkits_allow.is_empty());
    assert!(defaults.composio.offered_toolkits_deny.is_empty());
    let parsed: Config = toml::from_str(
        "[composio]\noffered_toolkits_allow = [\"gmail\"]\noffered_toolkits_deny = [\"slack\"]\n",
    )
    .expect("config parses");
    assert_eq!(parsed.composio.offered_toolkits_allow, ["gmail"]);
    assert_eq!(parsed.composio.offered_toolkits_deny, ["slack"]);
}
