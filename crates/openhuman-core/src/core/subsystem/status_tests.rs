//! Tests for the subsystem status wire row.

use super::*;

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
