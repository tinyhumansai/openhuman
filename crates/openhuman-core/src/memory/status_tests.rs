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

// --- The `subsystems` wire row ---------------------------------------------

#[test]
fn row_serializes_every_field_with_opaque_capabilities() {
    let row = SubsystemStatus {
        slot: "memory".to_string(),
        driver: "tinyhumans".to_string(),
        class: "external".to_string(),
        health: "ready".to_string(),
        health_reason: None,
        contract_version: "2.0".to_string(),
        capabilities: vec!["recall".to_string(), "some_future_op".to_string()],
        fell_back_from: None,
        last_error: None,
    };
    let value = serde_json::to_value(&row).expect("serializes");
    assert_eq!(value["slot"], "memory");
    assert_eq!(value["driver"], "tinyhumans");
    assert_eq!(value["contract_version"], "2.0");
    assert_eq!(
        value["capabilities"],
        serde_json::json!(["recall", "some_future_op"])
    );
    assert!(value["health_reason"].is_null());
    assert!(value["last_error"].is_null());
}

// --- The `subsystems` RPC namespace -----------------------------------------

#[test]
fn subsystem_catalog_matches_registered_controllers() {
    crate::core::all::expect_schema_namespace(&all_subsystems_controller_schemas(), "subsystems");
    crate::core::all::assert_schema_controller_parity(
        &all_subsystems_controller_schemas(),
        &all_subsystems_registered_controllers(),
        "status",
    );
}

#[tokio::test]
async fn handler_returns_a_subsystems_array_containing_the_memory_slot() {
    // The handler loads the process-default config, which reads the workspace
    // environment other tests point at temp dirs under this lock.
    let _env = crate::config::TEST_ENV_LOCK.lock().await;
    let value = handle_status(DispatchCtx::process_default(), Map::new())
        .await
        .expect("handler succeeds");
    let rows = value["subsystems"].as_array().expect("array");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["slot"], "memory");
    assert!(rows[0]["contract_version"].is_string());
    assert!(rows[0]["capabilities"].is_array());
}

// --- `openhuman subsystems` -------------------------------------------------

#[test]
fn help_flag_short_circuits_without_probing_a_driver() {
    run_subsystems_command(&["--help".to_string()]).expect("help succeeds");
    run_subsystems_command(&["-h".to_string()]).expect("short help succeeds");
}

#[test]
fn bare_invocation_renders_the_table() {
    run_subsystems_command(&[]).expect("table renders");
}

#[test]
fn namespace_has_a_registered_cli_adapter() {
    assert!(
        crate::core::all::cli_handler_for_namespace("subsystems").is_some(),
        "bare `openhuman subsystems` must reach the table, not the generic help"
    );
}
