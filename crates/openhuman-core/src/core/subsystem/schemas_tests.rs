use super::*;
use crate::core::all::assert_schema_controller_parity;
use crate::core::runtime::DispatchCtx;

#[test]
fn subsystem_catalog_matches_registered_controllers() {
    crate::core::all::expect_schema_namespace(&all_controller_schemas(), "subsystems");
    assert_schema_controller_parity(
        &all_controller_schemas(),
        &all_registered_controllers(),
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
