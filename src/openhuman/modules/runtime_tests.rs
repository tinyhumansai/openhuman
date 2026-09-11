//! Tests for the `tinyruntime` client facade.
//!
//! These exercise the parts that are this host's decisions rather than the
//! module's: which provider serves a language, how this host's configuration
//! becomes a request, and how a bus failure is classified. The module's own
//! behaviour — resolution order, digest verification, pooling — is tested in its
//! repository against its own suite, and re-asserting it here would only test
//! the mock.

use super::{
    pool_settings_for, provider_id, settings_for, RuntimeCallError, MODULE_ID, NODEJS_PROVIDER_ID,
    PYTHON_PROVIDER_ID,
};
use crate::openhuman::config::Config;
use crate::openhuman::modules::registry;
use tinyruntime_bus::Language;

#[test]
fn each_first_party_language_maps_to_its_provider_module() {
    assert_eq!(provider_id(&Language::nodejs()), Some(NODEJS_PROVIDER_ID));
    assert_eq!(provider_id(&Language::python()), Some(PYTHON_PROVIDER_ID));
}

#[test]
fn a_language_this_build_ships_no_provider_for_is_not_an_error_here() {
    // The router accepts whatever its own configuration routes, and an operator
    // may have put a third-party provider on the module search path. Refusing
    // here would make that impossible.
    assert_eq!(provider_id(&Language::new("ruby")), None);
}

#[test]
fn the_provider_records_named_here_exist_in_the_registry() {
    // A typo would surface as "unknown module" at the first tool call, long
    // after the change that caused it.
    for id in [MODULE_ID, NODEJS_PROVIDER_ID, PYTHON_PROVIDER_ID] {
        assert!(
            registry::find(id).is_some(),
            "no registry record for '{id}'"
        );
    }
}

#[test]
fn node_settings_come_from_the_node_config_block() {
    let mut config = Config::default();
    config.node.version = "v22.11.0".to_string();
    config.node.prefer_system = false;
    config.node.cache_dir = "/tmp/node-cache".to_string();

    let settings = settings_for(&config, &Language::nodejs());
    assert_eq!(settings.version, "v22.11.0");
    assert!(!settings.prefer_system);
    assert_eq!(settings.cache_dir, "/tmp/node-cache");
}

#[test]
fn python_settings_come_from_the_python_config_block() {
    let mut config = Config::default();
    config.runtime_python.minimum_version = "3.12".to_string();
    config.runtime_python.maximum_version = "3.15".to_string();
    config.runtime_python.preferred_command = "/usr/bin/python3.12".to_string();

    let settings = settings_for(&config, &Language::python());
    assert_eq!(settings.version, "3.12");
    assert_eq!(settings.maximum_version, "3.15");
    assert_eq!(settings.preferred_command, "/usr/bin/python3.12");
}

#[test]
fn the_two_languages_do_not_read_each_others_configuration() {
    // The bug this rules out: a Python request served under the Node version
    // pin, which would ask the Python channel for `v22.11.0`.
    let mut config = Config::default();
    config.node.version = "v22.11.0".to_string();
    config.runtime_python.minimum_version = "3.12".to_string();

    assert_eq!(
        settings_for(&config, &Language::nodejs()).version,
        "v22.11.0"
    );
    assert_eq!(settings_for(&config, &Language::python()).version, "3.12");
}

#[test]
fn a_disabled_language_is_carried_into_the_request_rather_than_refused_here() {
    // The module says why a language is unavailable; duplicating the check here
    // would mean two places to change and two messages to keep in step.
    let mut config = Config::default();
    config.node.enabled = false;
    assert!(!settings_for(&config, &Language::nodejs()).enabled);
}

#[test]
fn node_pools_by_default_and_python_does_not() {
    // Not an oversight: a pooled Node job runs in its own worker thread with a
    // fresh module graph, while a pooled Python job shares the interpreter with
    // every other job on that worker.
    let config = Config::default();
    assert!(pool_settings_for(&config, &Language::nodejs()).enabled);
    assert!(!pool_settings_for(&config, &Language::python()).enabled);
}

#[test]
fn turning_the_pool_off_wholesale_turns_it_off_for_every_language() {
    let mut config = Config::default();
    config.runtime_pool.enabled = false;
    assert!(!pool_settings_for(&config, &Language::nodejs()).enabled);
}

#[test]
fn pool_tuning_is_carried_from_this_hosts_configuration() {
    let mut config = Config::default();
    config.runtime_pool.node.max_workers = 4;
    config.runtime_pool.node.recycle_after_jobs = 25;

    let settings = pool_settings_for(&config, &Language::nodejs());
    assert_eq!(settings.max_workers, 4);
    assert_eq!(settings.recycle_after_jobs, 25);
}

#[test]
fn a_zero_worker_pool_is_clamped_before_it_leaves_this_host() {
    // A pool that can hold no workers would queue every job forever. The module
    // clamps too, but sending a zero would make the request a lie about what
    // this host asked for.
    let mut config = Config::default();
    config.runtime_pool.node.max_workers = 0;
    assert!(pool_settings_for(&config, &Language::nodejs()).max_workers >= 1);
}

#[test]
fn an_unloadable_module_and_a_bad_request_are_different_failures() {
    // Callers act on them differently: one disables a feature, the other is
    // worth reporting to whoever made the request.
    let unavailable = RuntimeCallError::Unavailable("no artifact for this host".to_string());
    let invalid = RuntimeCallError::InvalidRequest("no runtime provider for `ruby`".to_string());
    assert_ne!(unavailable, invalid);
    assert_eq!(unavailable.to_string(), "no artifact for this host");
}

#[test]
fn a_call_error_renders_as_its_message_alone() {
    // These are surfaced to models and users, so a variant name leaking into the
    // text would be noise in a chat transcript.
    for error in [
        RuntimeCallError::Unavailable("a".to_string()),
        RuntimeCallError::InvalidRequest("b".to_string()),
        RuntimeCallError::Failed("c".to_string()),
    ] {
        let rendered = error.to_string();
        assert!(rendered.len() <= 1, "got `{rendered}`");
    }
}
