//! Tests for the subsystem registry: bind, rebind, fallback-on-failure, and
//! health updates.

use std::cell::Cell;
use std::str::FromStr;

use serde_json::json;

use super::*;

const CONTRACT: (u16, u16) = (1, 0);

fn bound(slot: SubsystemSlot, id: &str, class: DriverClass) -> BoundDriver {
    BoundDriver::new(
        slot,
        id,
        class,
        ["core", "recall", "portability"].into_iter().collect(),
        CONTRACT,
    )
}

#[test]
fn subsystem_slot_as_str_matches_serde_representation() {
    for slot in SubsystemSlot::ALL {
        let encoded = serde_json::to_value(slot).expect("slot serializes");
        assert_eq!(encoded, json!(slot.as_str()), "mismatch for {slot:?}");
    }
}

#[test]
fn subsystem_slot_parse_round_trips_every_variant() {
    for slot in SubsystemSlot::ALL {
        assert_eq!(SubsystemSlot::parse(slot.as_str()), Ok(slot));
        assert_eq!(SubsystemSlot::from_str(slot.as_str()), Ok(slot));
        assert_eq!(slot.to_string(), slot.as_str());
    }
    let err = SubsystemSlot::parse("telepathy").expect_err("unknown slot rejected");
    assert!(
        err.contains("telepathy"),
        "message should name the input: {err}"
    );
}

#[test]
fn bind_records_the_driver_in_its_slot() {
    let mut registry = SubsystemRegistry::new();
    assert!(registry.is_empty());

    registry.bind(bound(
        SubsystemSlot::Memory,
        "tinycortex",
        DriverClass::Embedded,
    ));

    let driver = registry
        .get(SubsystemSlot::Memory)
        .expect("memory slot is bound");
    assert_eq!(driver.id, "tinycortex");
    assert_eq!(driver.class, DriverClass::Embedded);
    assert_eq!(driver.contract_version, CONTRACT);
    assert_eq!(driver.health, DriverHealth::Ready);
    assert!(driver.capabilities.contains("recall"));
    assert!(!driver.is_fallback());
    assert_eq!(registry.len(), 1);
    assert_eq!(registry.bound_slots(), vec![SubsystemSlot::Memory]);
    assert!(registry.get(SubsystemSlot::Voice).is_none());
}

#[test]
fn bind_returns_no_previous_driver_for_an_empty_slot() {
    let mut registry = SubsystemRegistry::new();
    let previous = registry.bind(bound(
        SubsystemSlot::Sandbox,
        "landlock",
        DriverClass::Embedded,
    ));
    assert!(previous.is_none());
}

#[test]
fn rebind_replaces_and_returns_the_previous_driver() {
    let mut registry = SubsystemRegistry::new();
    registry.bind(bound(
        SubsystemSlot::Memory,
        "tinycortex",
        DriverClass::Embedded,
    ));

    let previous = registry
        .bind(bound(
            SubsystemSlot::Memory,
            "supermemory",
            DriverClass::External,
        ))
        .expect("rebind returns the displaced driver");

    assert_eq!(previous.id, "tinycortex");
    assert_eq!(
        registry.get(SubsystemSlot::Memory).expect("still bound").id,
        "supermemory",
        "exactly one driver per slot — the second replaces the first"
    );
    assert_eq!(registry.len(), 1, "a rebind does not add a slot");
}

#[test]
fn rebind_does_not_disturb_other_slots() {
    let mut registry = SubsystemRegistry::new();
    registry.bind(bound(
        SubsystemSlot::Memory,
        "tinycortex",
        DriverClass::Embedded,
    ));
    registry.bind(bound(
        SubsystemSlot::Voice,
        "whisper",
        DriverClass::Embedded,
    ));
    registry.bind(bound(
        SubsystemSlot::Memory,
        "supermemory",
        DriverClass::External,
    ));

    assert_eq!(
        registry.get(SubsystemSlot::Voice).expect("bound").id,
        "whisper"
    );
    assert_eq!(registry.len(), 2);
}

#[test]
fn bind_with_fallback_binds_the_primary_when_it_constructs() {
    let mut registry = SubsystemRegistry::new();
    let primary: Result<BoundDriver, String> = Ok(bound(
        SubsystemSlot::Memory,
        "supermemory",
        DriverClass::External,
    ));

    let driver = registry.bind_with_fallback(SubsystemSlot::Memory, "supermemory", primary, || {
        bound(SubsystemSlot::Memory, "tinycortex", DriverClass::Embedded)
    });

    assert_eq!(driver.id, "supermemory");
    assert!(!driver.is_fallback());
    assert_eq!(driver.fell_back_from, None);
}

#[test]
fn bind_with_fallback_binds_the_fallback_when_the_primary_fails() {
    let mut registry = SubsystemRegistry::new();
    let primary: Result<BoundDriver, String> = Err("handshake refused".into());

    let driver = registry.bind_with_fallback(SubsystemSlot::Memory, "supermemory", primary, || {
        bound(SubsystemSlot::Memory, "tinycortex", DriverClass::Embedded)
    });

    assert_eq!(driver.id, "tinycortex");
    assert_eq!(driver.class, DriverClass::Embedded);
    assert_eq!(
        registry.get(SubsystemSlot::Memory).expect("bound").id,
        "tinycortex"
    );
}

#[test]
fn bind_with_fallback_records_fell_back_from_so_status_is_never_silent() {
    let mut registry = SubsystemRegistry::new();
    let primary: Result<BoundDriver, String> = Err("handshake refused".into());

    registry.bind_with_fallback(SubsystemSlot::Memory, "supermemory", primary, || {
        bound(SubsystemSlot::Memory, "tinycortex", DriverClass::Embedded)
    });

    let driver = registry.get(SubsystemSlot::Memory).expect("bound");
    assert!(driver.is_fallback());
    assert_eq!(driver.fell_back_from.as_deref(), Some("supermemory"));

    let encoded = serde_json::to_value(driver).expect("status record serializes");
    assert_eq!(
        encoded["fell_back_from"],
        json!("supermemory"),
        "the substitution must be visible in status output"
    );
}

#[test]
fn bind_with_fallback_does_not_construct_the_fallback_on_success() {
    let mut registry = SubsystemRegistry::new();
    let constructed = Cell::new(false);
    let primary: Result<BoundDriver, String> = Ok(bound(
        SubsystemSlot::Memory,
        "supermemory",
        DriverClass::External,
    ));

    registry.bind_with_fallback(SubsystemSlot::Memory, "supermemory", primary, || {
        constructed.set(true);
        bound(SubsystemSlot::Memory, "tinycortex", DriverClass::Embedded)
    });

    assert!(
        !constructed.get(),
        "the embedded default must not be constructed when the primary binds"
    );
}

#[test]
fn set_health_updates_only_the_named_slot() {
    let mut registry = SubsystemRegistry::new();
    registry.bind(bound(
        SubsystemSlot::Memory,
        "tinycortex",
        DriverClass::Embedded,
    ));
    registry.bind(bound(
        SubsystemSlot::Voice,
        "whisper",
        DriverClass::Embedded,
    ));

    assert!(registry.set_health(
        SubsystemSlot::Memory,
        DriverHealth::degraded("index rebuilding")
    ));

    assert_eq!(
        registry.get(SubsystemSlot::Memory).expect("bound").health,
        DriverHealth::degraded("index rebuilding")
    );
    assert_eq!(
        registry.get(SubsystemSlot::Voice).expect("bound").health,
        DriverHealth::Ready
    );
}

#[test]
fn set_health_returns_false_for_an_unbound_slot() {
    let mut registry = SubsystemRegistry::new();
    assert!(!registry.set_health(SubsystemSlot::Flows, DriverHealth::down("gone")));
    assert!(registry.is_empty());
}

#[test]
fn registry_iterates_in_slot_declaration_order() {
    let mut registry = SubsystemRegistry::new();
    // Bind out of declaration order on purpose.
    for slot in [
        SubsystemSlot::Voice,
        SubsystemSlot::Memory,
        SubsystemSlot::Flows,
        SubsystemSlot::Inference,
    ] {
        registry.bind(bound(slot, "null", DriverClass::Null));
    }

    let order: Vec<SubsystemSlot> = registry.iter().map(|driver| driver.slot).collect();
    assert_eq!(
        order,
        vec![
            SubsystemSlot::Memory,
            SubsystemSlot::Inference,
            SubsystemSlot::Flows,
            SubsystemSlot::Voice,
        ]
    );
    assert_eq!(registry.bound_slots(), order);
}
