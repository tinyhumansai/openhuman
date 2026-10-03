use super::*;
use crate::memory::test_fixtures::config_in;
use crate::security::credentials::api_key::store_api_key;

fn off_reason(binding: Binding) -> String {
    match binding {
        Binding::Off { reason, .. } => reason,
        Binding::On(bound) => panic!("expected off, bound {}", bound.id),
    }
}

#[test]
fn tinyhumans_without_a_credential_is_off() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.engine = TINYHUMANS_ENGINE.to_string();
    config
        .memory
        .engines
        .entry(TINYHUMANS_ENGINE.to_string())
        .or_default()
        .endpoint = Some("https://memory.example.test".to_string());
    let binding = resolve(&config);
    assert!(!binding.is_on());
    assert!(!is_on(&config));
    match binding {
        Binding::Off {
            engine, endpoint, ..
        } => {
            assert_eq!(engine.as_deref(), Some(TINYHUMANS_ENGINE));
            assert_eq!(endpoint.as_deref(), Some("https://memory.example.test"));
        }
        Binding::On(_) => unreachable!(),
    }
    let error = resolve(&config).engine().unwrap_err();
    assert_eq!(error.code(), super::super::error::MEMORY_OFF);
}

#[test]
fn tinyhumans_with_the_host_credential_binds_and_caches() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config
        .memory
        .engines
        .entry(TINYHUMANS_ENGINE.to_string())
        .or_default()
        .endpoint = Some("https://memory.example.test".to_string());
    store_api_key(&config, "test-api-key-not-real").unwrap();
    assert!(has_key(&config, TINYHUMANS_ENGINE));

    let first = resolve(&config).engine().expect("bound");
    assert_eq!(first.id, TINYHUMANS_ENGINE);
    assert_eq!(first.endpoint, "https://memory.example.test");
    let second = resolve(&config).engine().expect("bound again");
    assert!(
        Arc::ptr_eq(&first.engine, &second.engine),
        "cached engine reused"
    );
}

#[test]
fn unknown_and_blank_engine_ids_are_off() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.engine = "nope".to_string();
    assert!(off_reason(resolve(&config)).contains("not available"));
    config.memory.engine = "  ".to_string();
    assert!(off_reason(resolve(&config)).contains("no memory engine"));
    assert!(!has_key(&config, "nope"));
}

#[test]
fn cortexdb_is_off_until_a_key_is_stored_and_rebuilds_on_a_new_key() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.engine = CORTEXDB_ENGINE.to_string();
    let reason = off_reason(resolve(&config));
    assert!(reason.contains("CortexDB API key"), "{reason}");
    assert!(!has_key(&config, CORTEXDB_ENGINE));

    store_cortexdb_key(&config, "  cdb-key-one  ").unwrap();
    assert_eq!(
        read_cortexdb_key(&config).unwrap().as_deref(),
        Some("cdb-key-one")
    );
    assert!(has_key(&config, CORTEXDB_ENGINE));
    let first = resolve(&config).engine().expect("bound");
    assert_eq!(first.id, CORTEXDB_ENGINE);
    assert_eq!(first.endpoint, tinymemory::cortex::CORTEX_API_ENDPOINT);

    store_cortexdb_key(&config, "cdb-key-two").unwrap();
    let second = resolve(&config).engine().expect("rebound");
    assert!(
        !Arc::ptr_eq(&first.engine, &second.engine),
        "a new key builds a new engine"
    );

    assert!(clear_cortexdb_key(&config).unwrap());
    assert!(!is_on(&config));
}

#[test]
fn a_blank_cortexdb_key_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let error = store_cortexdb_key(&config, "   ").unwrap_err();
    assert_eq!(error.code(), super::super::error::INVALID_REQUEST);
}

#[test]
fn the_cache_fingerprint_never_contains_the_key() {
    let digest = key_digest("super-secret-key");
    assert_eq!(digest.len(), 16);
    assert!(!digest.contains("secret"));
    assert_eq!(digest, key_digest("super-secret-key"));
    assert_ne!(digest, key_digest("another-key"));
}

#[tokio::test]
async fn host_bearer_reads_the_credential_per_request() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let source = HostBearer {
        config: Arc::new(config.clone()),
    };
    let error = source.bearer().await.unwrap_err();
    assert!(matches!(error, tinymemory::Error::Unauthorized(_)));

    store_api_key(&config, "test-api-key-not-real").unwrap();
    assert_eq!(source.bearer().await.unwrap(), "test-api-key-not-real");
}

#[test]
fn an_installed_test_engine_wins_and_is_reported_on() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    assert!(!is_on(&config));
    crate::memory::test_fixtures::bind_reference(&config);
    let bound = resolve(&config).engine().unwrap();
    assert_eq!(bound.id, "reference");
    assert!(format!("{bound:?}").contains("reference"));
}
