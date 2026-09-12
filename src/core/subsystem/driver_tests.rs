//! Tests for the generic driver vocabulary.
//!
//! Note: this is the **only** file under `src/core/subsystem/` that may name
//! `tinycortex_api`, and it does so purely as a drift witness — see
//! `driver_health_shape_matches_memory_health_one_for_one` and
//! `every_memory_contract_capability_string_maps_into_driver_capabilities`.
//! The production modules mention it only in prose; they must never *depend*
//! on it, which is checkable with:
//!
//! ```text
//! grep -rn '^ *use .*tinycortex_api' src/core/subsystem/driver.rs \
//!     src/core/subsystem/registry.rs src/core/subsystem/mod.rs   # no output
//! ```

use std::str::FromStr;

use serde_json::json;

use super::*;

#[test]
fn driver_class_as_str_matches_serde_representation() {
    for class in DriverClass::ALL {
        let encoded = serde_json::to_value(class).expect("class serializes");
        assert_eq!(encoded, json!(class.as_str()), "mismatch for {class:?}");
    }
}

#[test]
fn driver_class_parse_round_trips_every_variant() {
    for class in DriverClass::ALL {
        assert_eq!(DriverClass::parse(class.as_str()), Ok(class));
        assert_eq!(DriverClass::from_str(class.as_str()), Ok(class));
        assert_eq!(class.to_string(), class.as_str());
    }
}

#[test]
fn driver_class_parse_rejects_unknown_with_the_input_in_the_message() {
    let err = DriverClass::parse("sidecar").expect_err("unknown class rejected");
    assert!(
        err.contains("sidecar"),
        "message should name the input: {err}"
    );
}

#[test]
fn driver_health_is_usable_is_false_only_when_down() {
    assert!(DriverHealth::Ready.is_usable());
    assert!(DriverHealth::degraded("index rebuilding").is_usable());
    assert!(!DriverHealth::down("connection refused").is_usable());
}

#[test]
fn driver_health_serializes_with_a_stable_status_discriminant() {
    assert_eq!(
        serde_json::to_value(DriverHealth::Ready).expect("ready serializes"),
        json!({ "status": "ready" })
    );
    assert_eq!(
        serde_json::to_value(DriverHealth::degraded("slow")).expect("degraded serializes"),
        json!({ "status": "degraded", "reason": "slow" })
    );
    assert_eq!(
        serde_json::to_value(DriverHealth::down("refused")).expect("down serializes"),
        json!({ "status": "down", "reason": "refused" })
    );

    let decoded: DriverHealth =
        serde_json::from_value(json!({ "status": "degraded", "reason": "slow" }))
            .expect("degraded deserializes");
    assert_eq!(decoded, DriverHealth::degraded("slow"));
}

#[test]
fn driver_health_display_includes_the_reason() {
    assert_eq!(DriverHealth::Ready.to_string(), "ready");
    assert_eq!(DriverHealth::Ready.reason(), None);
    assert_eq!(
        DriverHealth::down("connection refused").to_string(),
        "down: connection refused"
    );
    assert_eq!(
        DriverHealth::degraded("slow").reason(),
        Some("slow"),
        "reason is readable for status output"
    );
}

/// Drift guard for the boundary conversion that lands with the memory adapter.
///
/// The contract crate's `health` module states that `MemoryHealth` is shaped
/// one-for-one against the kernel's `Ready | Degraded { reason } | Down
/// { reason }` so the conversion is trivial and lossless. This asserts that,
/// pairwise, on the serialized form — so a fourth state, a renamed
/// discriminant, or an extra field on either side fails here rather than
/// silently making the conversion partial.
#[test]
fn driver_health_shape_matches_memory_health_one_for_one() {
    use tinymemory_api::health::MemoryHealth;

    let pairs: Vec<(MemoryHealth, DriverHealth)> = vec![
        (MemoryHealth::Ready, DriverHealth::Ready),
        (
            MemoryHealth::degraded("index rebuilding"),
            DriverHealth::degraded("index rebuilding"),
        ),
        (
            MemoryHealth::down("connection refused"),
            DriverHealth::down("connection refused"),
        ),
    ];
    assert_eq!(pairs.len(), 3, "both enums have exactly three states");

    for (contract, kernel) in pairs {
        assert_eq!(contract.as_str(), kernel.as_str());
        assert_eq!(contract.reason(), kernel.reason());
        assert_eq!(contract.is_usable(), kernel.is_usable());
        assert_eq!(
            serde_json::to_value(&contract).expect("contract health serializes"),
            serde_json::to_value(&kernel).expect("kernel health serializes"),
        );
    }
}

#[test]
fn driver_capabilities_round_trips_through_json() {
    let caps = DriverCapabilities::empty()
        .with("core")
        .with("recall")
        .with("portability");

    let encoded = serde_json::to_value(&caps).expect("capabilities serialize");
    let decoded: DriverCapabilities =
        serde_json::from_value(encoded.clone()).expect("capabilities deserialize");

    assert_eq!(decoded, caps);
    assert_eq!(encoded, json!(["core", "portability", "recall"]));
}

#[test]
fn driver_capabilities_collapses_duplicates() {
    let caps: DriverCapabilities = ["core", "recall", "core"].into_iter().collect();
    assert_eq!(caps.len(), 2);
    assert!(caps.contains("core"));
    assert!(caps.contains("recall"));
    assert!(!caps.contains("tree"));

    let mut caps = caps;
    caps.remove("recall");
    assert_eq!(caps.len(), 1);
    caps.remove("recall");
    assert_eq!(caps.len(), 1, "remove is idempotent");
}

#[test]
fn driver_capabilities_serializes_as_an_array_of_strings() {
    assert_eq!(
        serde_json::to_value(DriverCapabilities::empty()).expect("empty serializes"),
        json!([])
    );
    assert!(DriverCapabilities::empty().is_empty());

    let mut caps = DriverCapabilities::empty();
    caps.extend(["tree", "core"]);
    assert_eq!(
        serde_json::to_value(&caps).expect("serializes"),
        json!(["core", "tree"]),
        "a set has no order; the backing BTreeSet emits lexicographic order"
    );
}

#[test]
fn driver_capabilities_contains_all_is_subset_semantics() {
    let advertised: DriverCapabilities = ["core", "recall", "portability", "tree"]
        .into_iter()
        .collect();
    let mandatory: DriverCapabilities = ["core", "recall", "portability"].into_iter().collect();

    assert!(advertised.contains_all(&mandatory));
    assert!(!mandatory.contains_all(&advertised));
    assert!(advertised.contains_all(&DriverCapabilities::empty()));
}

/// The kernel's opaque-string set must be able to carry every family the memory
/// contract defines, losslessly and through the wire form — that is what makes
/// the future boundary conversion total without the kernel knowing what a
/// memory capability is.
#[test]
fn every_memory_contract_capability_string_maps_into_driver_capabilities() {
    use tinymemory_api::capabilities::Capability;

    let caps: DriverCapabilities = Capability::ALL.iter().map(|cap| cap.as_str()).collect();

    assert_eq!(caps.len(), Capability::ALL.len());
    // A literal, so adding a family forces a look at this test rather than
    // sliding past it. 13 → 17 when the port added People, Chunks, Retrieval
    // and Profile, then 18 with Episodic, then 20 when tinymemory v1.7.0 added
    // SourceSync and CodingSessions, then 21 with v1.13.2 adding Scoring,
    // then 26 with v1.13.7's typed-ingestion round (Document/Conversation/
    // Learning/Event ingest and Answer). The assertion above is the
    // load-bearing one: it says the mapping is lossless, which is what makes
    // the kernel's opaque-string set able to carry the contract without
    // knowing what a memory capability is.
    assert_eq!(caps.len(), 26);
    assert!(
        caps.contains("tool_memory"),
        "the one non-identity snake_case family must survive"
    );
    for cap in Capability::ALL {
        assert!(caps.contains(cap.as_str()), "missing {cap}");
    }

    let encoded = serde_json::to_value(&caps).expect("serializes");
    let decoded: DriverCapabilities = serde_json::from_value(encoded).expect("deserializes");
    assert_eq!(decoded, caps);
}
