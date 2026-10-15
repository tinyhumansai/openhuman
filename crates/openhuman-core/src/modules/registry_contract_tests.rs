//! Registry metadata against each module's bus contract constants.

use super::{component_for, find, ALL, COMPONENT_OF};

/// Modules that deliberately have no `[components]` switch: security
/// enforcement must not be switchable off through configuration.
const UNSWITCHABLE: &[&str] = &["tinysecurity"];

/// The interface and object path each contract crate publishes, keyed by the
/// module id the registry uses. This is written out from the contract
/// constants, not from the records, so a record that drifts from its contract
/// (or a contract that is renamed upstream) fails here rather than at the first
/// bus call. Records behind an optional contract crate are listed under the
/// same gate as the crate.
fn contract_names() -> Vec<(&'static str, &'static str, &'static str)> {
    #[allow(unused_mut)]
    let mut names = vec![
        (
            "tinycomputer",
            tinycomputer_bus::names::INTERFACE,
            tinycomputer_bus::names::OBJECT_PATH,
        ),
        (
            "tinysearch",
            tinysearch_bus::names::INTERFACE,
            tinysearch_bus::names::OBJECT_PATH,
        ),
        (
            "tinyjuice",
            tinyjuice_bus::names::BUS_NAME,
            tinyjuice_bus::names::OBJECT_PATH,
        ),
        (
            "tinymcp",
            tinymcp_bus::names::INTERFACE,
            tinymcp_bus::names::OBJECT_PATH,
        ),
        (
            "tinyconnectors",
            tinyconnectors_bus::names::INTERFACE,
            tinyconnectors_bus::names::OBJECT_PATH,
        ),
        (
            "tinychannels",
            tinychannels_bus::names::BUS_NAME,
            tinychannels_bus::names::OBJECT_PATH,
        ),
    ];
    #[cfg(feature = "modules")]
    names.push(("tinybox", tinybox_bus::INTERFACE, tinybox_bus::OBJECT_PATH));
    #[cfg(feature = "documents")]
    names.push((
        "tinydocs",
        tinydocs_bus::names::BUS_NAME,
        tinydocs_bus::names::OBJECT_PATH,
    ));
    #[cfg(feature = "web3")]
    names.push((
        "tinywallet",
        tinywallet_bus::names::BUS_NAME,
        tinywallet_bus::names::OBJECT_PATH,
    ));
    #[cfg(feature = "voice")]
    names.push((
        "tinyvoice",
        tinyvoice_bus::names::BUS_NAME,
        tinyvoice_bus::names::OBJECT_PATH,
    ));
    #[cfg(feature = "hosting")]
    names.push((
        "tinyhosts",
        tinyhosts_bus::BUS_NAME,
        tinyhosts_bus::OBJECT_PATH,
    ));
    #[cfg(feature = "tinymemes")]
    names.push((
        "tinymemes",
        tinymemes_bus::STATE_INTERFACE,
        tinymemes_bus::STATE_PATH,
    ));
    #[cfg(feature = "security-module")]
    names.push((
        "tinysecurity",
        tinysecurity_bus::names::INTERFACE,
        tinysecurity_bus::names::OBJECT_PATH,
    ));
    names
}

#[test]
fn every_record_matches_its_contract_constants() {
    let contracts = contract_names();
    assert_eq!(contracts.len(), ALL.len(), "a record has no contract entry");
    for (id, bus_name, object_path) in contracts {
        let record = find(id).unwrap_or_else(|| panic!("{id} has no registry record"));
        assert_eq!(
            record.bus_name, bus_name,
            "{id} bus name drifted from its contract"
        );
        assert_eq!(
            record.object_path, object_path,
            "{id} object path drifted from its contract"
        );
    }
}

#[test]
fn a_drifted_contract_constant_is_reported_as_a_mismatch() {
    // The comparison must be able to fail: a fixture whose constant differs
    // from the record (as a renamed upstream interface would) is not equal.
    let record = find("tinymcp").unwrap();
    let drifted = format!("{}Renamed", tinymcp_bus::names::INTERFACE);
    assert_ne!(record.bus_name, drifted);
}

#[cfg(all(
    feature = "documents",
    feature = "web3",
    feature = "voice",
    feature = "hosting",
    feature = "tinymemes"
))]
#[test]
fn a_product_feature_build_carries_all_twelve_records() {
    // TinySecurity is a thirteenth, always-on record whenever its module is
    // built in (`security-module`); it has no component switch.
    let expected = 12 + usize::from(cfg!(feature = "security-module"));
    assert_eq!(ALL.len(), expected);
    for id in [
        "tinycomputer",
        "tinysearch",
        "tinydocs",
        "tinywallet",
        "tinyjuice",
        "tinyvoice",
        "tinymcp",
        "tinyconnectors",
        "tinybox",
        "tinychannels",
        "tinyhosts",
        "tinymemes",
    ] {
        assert!(find(id).is_some(), "{id} missing from the registry");
    }
}

#[test]
fn every_record_maps_to_a_real_component() {
    for record in ALL {
        if UNSWITCHABLE.contains(&record.id) {
            assert!(
                component_for(record.id).is_none(),
                "'{}' is security enforcement and must have no [components] switch",
                record.id
            );
            continue;
        }
        let component = component_for(record.id)
            .unwrap_or_else(|| panic!("module '{}' has no component mapping", record.id));
        assert!(
            crate::core::runtime::COMPONENT_NAMES.contains(&component),
            "'{component}' is not a [components] name"
        );
    }
    for (id, _) in COMPONENT_OF {
        assert!(
            ALL.iter().any(|r| r.id == *id)
                || !cfg!(feature = "modules")
                || matches!(
                    *id,
                    "tinydocs" | "tinywallet" | "tinyvoice" | "tinyhosts" | "tinymemes"
                ),
            "mapping for unknown module '{id}'"
        );
    }
}
