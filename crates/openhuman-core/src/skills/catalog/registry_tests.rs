use super::*;

use std::collections::HashMap;

fn config_from(vars: &[(&str, &str)], home: Option<&Path>) -> RegistryConfig {
    let vars: HashMap<String, String> = vars
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    RegistryConfig::from_lookup(|name| vars.get(name).cloned(), home.map(Path::to_path_buf))
}

#[test]
fn defaults_read_the_public_index_into_the_home_cache() {
    let home = tempfile::tempdir().unwrap();
    let config = config_from(&[], Some(home.path()));
    assert_eq!(config.catalog_url, None);
    assert_eq!(config.download_base, None);
    assert!(!config.allow_loopback_http);
    assert_eq!(
        config.cache_dir,
        Some(home.path().join(".openhuman").join("skill-registry"))
    );
    assert_eq!(config.source().url(), HermesIndexSource::hermes().url());
    assert!(!config.policy().allow_loopback_http);
    assert!(config.policy().user_agent.starts_with("openhuman-core/"));
}

#[test]
fn environment_overrides_map_onto_the_registry() {
    let config = config_from(
        &[
            (CATALOG_URL_ENV, " http://127.0.0.1:9/skills.json "),
            (DOWNLOAD_BASE_URL_ENV, "http://127.0.0.1:9/skills"),
            ("OPENHUMAN_SKILL_INSTALL_ALLOW_LOCAL_HTTP", "1"),
            (CACHE_DIR_ENV, "/tmp/registry-cache"),
        ],
        None,
    );
    assert_eq!(
        config.catalog_url.as_deref(),
        Some("http://127.0.0.1:9/skills.json")
    );
    assert_eq!(config.source().url(), "http://127.0.0.1:9/skills.json");
    assert_eq!(
        config.download_base.as_deref(),
        Some("http://127.0.0.1:9/skills")
    );
    assert!(config.allow_loopback_http);
    assert!(config.policy().allow_loopback_http);
    assert_eq!(config.cache_dir, Some(PathBuf::from("/tmp/registry-cache")));
}

#[test]
fn blank_overrides_are_ignored_and_only_one_enables_local_http() {
    let config = config_from(
        &[
            (CATALOG_URL_ENV, "  "),
            (DOWNLOAD_BASE_URL_ENV, ""),
            ("OPENHUMAN_SKILL_INSTALL_ALLOW_LOCAL_HTTP", "true"),
            (CACHE_DIR_ENV, " "),
        ],
        None,
    );
    assert_eq!(config.catalog_url, None);
    assert_eq!(config.download_base, None);
    assert!(!config.allow_loopback_http, "only `1` enables local http");
    assert_eq!(config.cache_dir, None, "no home and no override");
}

#[test]
fn limits_allow_a_whole_catalog_in_one_page() {
    let limits = RegistryConfig::limits();
    assert_eq!(limits.max_page_size, limits.max_entries);
}

#[test]
fn the_handle_is_reused_for_one_config_and_rebuilt_for_another() {
    let _env = crate::skills::catalog::TEST_ENV_LOCK.blocking_lock();
    reset_for_tests();
    let first_dir = tempfile::tempdir().unwrap();
    let second_dir = tempfile::tempdir().unwrap();
    let first = config_from(&[(CACHE_DIR_ENV, first_dir.path().to_str().unwrap())], None);
    let second = config_from(
        &[(CACHE_DIR_ENV, second_dir.path().to_str().unwrap())],
        None,
    );

    let a = registry_for(first.clone());
    let b = registry_for(first);
    assert!(Arc::ptr_eq(&a, &b), "same config shares one registry");
    let c = registry_for(second);
    assert!(!Arc::ptr_eq(&a, &c), "a new config builds a new registry");
    reset_for_tests();
}

#[test]
fn building_removes_the_legacy_cache_file() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join(LEGACY_CACHE_FILE);
    std::fs::write(&legacy, r#"{"entries":[],"fetched_at_epoch":0}"#).unwrap();
    let config = config_from(&[(CACHE_DIR_ENV, dir.path().to_str().unwrap())], None);
    let _registry = config.build();
    assert!(!legacy.exists(), "the pre-registry cache is dropped");
}

#[test]
fn a_failed_refresh_cools_down_for_45_seconds() {
    assert_eq!(REFRESH_COOLDOWN, std::time::Duration::from_secs(45));
    assert_eq!(RegistryConfig::timeouts().cooldown, REFRESH_COOLDOWN);
    assert_eq!(registry_timeouts().cooldown, REFRESH_COOLDOWN);
    let defaults = RegistryTimeouts::default();
    let timeouts = RegistryConfig::timeouts();
    assert_eq!(timeouts.document, defaults.document);
    assert_eq!(timeouts.catalog, defaults.catalog);
}
