//! Tests for the wire projection over a bound driver.

use super::*;
use crate::core::subsystem::driver::{DriverCapabilities, DriverClass};
use crate::core::subsystem::registry::{BoundDriver, SubsystemRegistry, SubsystemSlot};

fn embedded_memory() -> BoundDriver {
    BoundDriver::new(
        SubsystemSlot::Memory,
        "tinycortex",
        DriverClass::Embedded,
        DriverCapabilities::empty()
            .with("core")
            .with("recall")
            .with("portability"),
        (1, 0),
    )
}

#[test]
fn projection_reports_slot_driver_class_and_contract_version() {
    let status = SubsystemStatus::from_bound(&embedded_memory());
    assert_eq!(status.slot, "memory");
    assert_eq!(status.driver, "tinycortex");
    assert_eq!(status.class, "embedded");
    assert_eq!(status.contract_version, "1.0");
    assert_eq!(status.health, "ready");
    assert_eq!(status.health_reason, None);
    assert_eq!(status.fell_back_from, None);
    assert_eq!(status.last_error, None);
}

#[test]
fn projection_reports_the_advertised_capability_list() {
    let status = SubsystemStatus::from_bound(&embedded_memory());
    // `DriverCapabilities` is a set, so ordering is lexicographic, not the
    // contract's declaration order. Asserted explicitly so a future switch to
    // declaration order is a deliberate change, not a silent one.
    assert_eq!(status.capabilities, vec!["core", "portability", "recall"]);
}

#[test]
fn live_health_overrides_the_cached_record() {
    let status = SubsystemStatus::from_bound_with_health(
        &embedded_memory(),
        DriverHealth::degraded("vector index rebuilding"),
    );
    assert_eq!(status.health, "degraded");
    assert_eq!(
        status.health_reason.as_deref(),
        Some("vector index rebuilding")
    );
}

#[test]
fn a_fallback_binding_surfaces_the_driver_it_replaced() {
    let mut driver = BoundDriver::new(
        SubsystemSlot::Memory,
        "null",
        DriverClass::Null,
        DriverCapabilities::empty(),
        (1, 0),
    );
    driver.fell_back_from = Some("supermemory".to_string());
    let status = SubsystemStatus::from_bound(&driver)
        .with_last_error(Some("external driver is untrusted".to_string()));
    assert_eq!(status.fell_back_from.as_deref(), Some("supermemory"));
    assert_eq!(
        status.last_error.as_deref(),
        Some("external driver is untrusted")
    );
    assert!(status.capabilities.is_empty());
    assert_eq!(status.class, "null");
}

#[test]
fn capabilities_serialize_as_a_flat_array_of_strings() {
    let value = serde_json::to_value(SubsystemStatus::from_bound(&embedded_memory()))
        .expect("status serializes");
    let caps = value["capabilities"].as_array().expect("array");
    assert!(caps.iter().all(serde_json::Value::is_string));
    assert_eq!(value["class"], "embedded");
    assert_eq!(value["health"], "ready");
    // Health is flattened, never nested under a `status` discriminant object.
    assert!(value["health"].is_string());
}

#[test]
fn registry_projection_follows_slot_declaration_order() {
    let mut registry = SubsystemRegistry::new();
    registry.bind(BoundDriver::new(
        SubsystemSlot::Inference,
        "openai",
        DriverClass::External,
        DriverCapabilities::empty(),
        (1, 0),
    ));
    registry.bind(embedded_memory());

    let rows = registry_status(&registry);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].slot, "memory");
    assert_eq!(rows[1].slot, "inference");
}

#[test]
fn contract_version_formats_as_major_dot_minor() {
    assert_eq!(format_contract_version((2, 7)), "2.7");
}
