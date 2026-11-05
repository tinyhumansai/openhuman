//! Pure helpers for the agent37 backend: the `timeout` wrapping, whole-second
//! rounding, the box limits, and the UTF-8 environment projection. The command path itself goes
//! through the TinyBox module and is exercised by integration tests, not here.
#![cfg(feature = "modules")]

use super::*;
use std::collections::HashMap;
use std::ffi::OsString;
use std::time::Duration;

#[test]
fn timeout_argv_wraps_the_command_for_the_box_to_stop() {
    assert_eq!(
        timeout_argv(30, "echo hi"),
        vec![
            "timeout".to_owned(),
            "-k".to_owned(),
            "5".to_owned(),
            "30".to_owned(),
            "sh".to_owned(),
            "-c".to_owned(),
            "echo hi".to_owned(),
        ]
    );
}

#[test]
fn whole_seconds_rounds_up_and_never_asks_for_none() {
    assert_eq!(whole_seconds(Duration::from_secs(0)), 1);
    assert_eq!(whole_seconds(Duration::from_millis(1)), 1);
    assert_eq!(whole_seconds(Duration::from_millis(1500)), 2);
    assert_eq!(whole_seconds(Duration::from_secs(5)), 5);
}

#[test]
fn utf8_env_keeps_utf8_pairs_and_drops_the_rest() {
    let mut env = HashMap::new();
    env.insert(OsString::from("KEY"), OsString::from("value"));
    let out = utf8_env(&env);
    assert_eq!(out.get("KEY").map(String::as_str), Some("value"));
    assert_eq!(out.len(), 1);
}

#[test]
fn local_only_privacy_forbids_agent37() {
    let mut config = Config::default();
    // Standard (the default) allows agent37's off-device execution.
    assert!(!local_only_forbids_agent37(&config));
    config.privacy.mode = crate::config::PrivacyMode::LocalOnly;
    assert!(local_only_forbids_agent37(&config));
    // Sensitive is not local-only, so it still allows agent37.
    config.privacy.mode = crate::config::PrivacyMode::Sensitive;
    assert!(!local_only_forbids_agent37(&config));
}

#[test]
fn box_resources_come_from_the_agent37_runtime_config() {
    let mut config = Config::default();
    config.runtime.docker.cpu_limit = Some(8.0);
    config.runtime.docker.memory_limit_mb = Some(1);
    config.runtime.agent37.cpu_millis = 4_000;
    config.runtime.agent37.memory_mb = 16_384;

    let resources = box_resources(&config);

    assert_eq!(resources.cpu_millis, 4_000);
    assert_eq!(resources.memory_bytes, 16_384 * 1024 * 1024);
}

#[test]
fn box_resources_ask_for_the_maximum_when_memory_overflows() {
    let mut config = Config::default();
    config.runtime.agent37.memory_mb = u64::MAX;

    assert_eq!(box_resources(&config).memory_bytes, u64::MAX);
}
