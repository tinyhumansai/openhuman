use super::*;
use crate::memory::test_fixtures::{bind_reference, config_in};

#[test]
fn upstream_engine_status_maps_health_reason_and_capabilities() {
    let cases = [
        (tinymemory_api::EngineStatusHealth::Ready, None, "ready"),
        (
            tinymemory_api::EngineStatusHealth::Degraded,
            Some("partial outage".to_string()),
            "degraded",
        ),
        (
            tinymemory_api::EngineStatusHealth::Down,
            Some("service unavailable".to_string()),
            "down",
        ),
    ];

    for (health, reason, expected_health) in cases {
        let input_contract_version = "memory-contract-fixture-2.0";
        let status = tinymemory_api::EngineStatus {
            health,
            reason: reason.clone(),
            capabilities: vec!["recall", "store"],
            contract_version: input_contract_version,
        };
        let row = subsystem_status_for_engine("cortexdb".into(), status);
        assert_eq!(row.slot, "memory");
        assert_eq!(row.driver, "cortexdb");
        assert_eq!(row.class, "external");
        assert_eq!(row.health, expected_health);
        assert_eq!(row.health_reason, reason);
        assert_eq!(row.contract_version, input_contract_version);
        assert_eq!(row.capabilities, vec!["recall", "store"]);
        assert!(row.fell_back_from.is_none());
        assert!(row.last_error.is_none());
    }
}

#[tokio::test]
async fn memory_off_reports_a_null_row_with_the_reason() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let row = subsystem_status(&config).await;
    assert_eq!(row.slot, "memory");
    assert_eq!(row.driver, "null");
    assert_eq!(row.class, "null");
    assert_eq!(row.health, "down");
    assert!(row.health_reason.is_some());
    assert_eq!(row.contract_version, tinymemory_api::CONTRACT_VERSION);
    assert!(row.capabilities.is_empty());
    assert_eq!(
        row.fell_back_from.as_deref(),
        Some(engine::TINYHUMANS_ENGINE),
        "the configured engine is named as what memory fell back from"
    );
    assert!(row.last_error.is_none());
}

#[tokio::test]
async fn bound_engine_reports_ready_with_every_operation() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    bind_reference(&config);
    let row = subsystem_status(&config).await;
    assert_eq!(row.slot, "memory");
    assert_eq!(row.driver, "reference");
    assert_eq!(row.class, "external");
    assert_eq!(row.health, "ready");
    assert!(row.health_reason.is_none());
    assert_eq!(
        row.capabilities,
        vec!["recall", "fetch", "store", "forget", "list"]
    );
    assert!(row.fell_back_from.is_none());
}

#[tokio::test]
async fn an_unknown_engine_id_is_off_without_a_fallback_name() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.engine = "nonexistent".into();
    let row = subsystem_status(&config).await;
    assert_eq!(row.driver, "null");
    assert!(row.fell_back_from.is_none());
}
