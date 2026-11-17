//! Registry metadata against each module's bus contract constants.

use super::{component_for, find, ALL, COMPONENT_OF};

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
        // The Authoring identity the module registers under; the scheduler is
        // a second object on it (pinned by `the_flows_record_serves_the_schedule_object`).
        (
            "tinyflows",
            tinyflows_bus::BUS_NAME,
            tinyflows_bus::OBJECT_PATH,
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
    #[cfg(feature = "security-module")]
    names.push((
        "tinysecurity",
        tinysecurity_bus::names::INTERFACE,
        tinysecurity_bus::names::OBJECT_PATH,
    ));
    #[cfg(feature = "hosting")]
    names.push((
        "tinyhosts",
        tinyhosts_bus::BUS_NAME,
        tinyhosts_bus::OBJECT_PATH,
    ));
    #[cfg(feature = "inbound-queues")]
    names.push((
        "tinyeventqueues",
        tinyeventqueues_bus::INTERFACE,
        tinyeventqueues_bus::OBJECT_PATH,
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
    feature = "inbound-queues",
    feature = "security-module"
))]
#[test]
fn a_product_feature_build_carries_all_fourteen_records() {
    assert_eq!(ALL.len(), 14);
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
        "tinyflows",
        "tinyhosts",
        "tinyeventqueues",
        "tinysecurity",
    ] {
        assert!(find(id).is_some(), "{id} missing from the registry");
    }
}

#[test]
fn every_record_maps_to_a_real_component() {
    for record in ALL {
        // TinySecurity has no product component yet; the global module switch
        // is its only host gate until its callers move behind the bus.
        if record.id == "tinysecurity" {
            assert_eq!(component_for(record.id), None);
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
                    "tinydocs" | "tinywallet" | "tinyvoice" | "tinyhosts" | "tinyeventqueues"
                ),
            "mapping for unknown module '{id}'"
        );
    }
}

/// The scheduler is a second interface on the same module, and the host calls
/// it by the contract's own names: the record carries the Authoring identity,
/// and the Schedule object and callback interfaces are distinct from it and from
/// each other.
#[test]
fn the_flows_record_serves_the_schedule_object() {
    use tinyflows_bus::schedule;
    let record = find("tinyflows").expect("tinyflows is registered");
    assert_eq!(record.bus_name, schedule::BUS_NAME);
    assert_ne!(record.object_path, schedule::OBJECT_PATH);
    assert_ne!(schedule::INTERFACE, schedule::CALLBACK_INTERFACE);
    assert_ne!(schedule::INTERFACE, schedule::DOCUMENT_INTERFACE);
    assert_eq!(schedule::CONTRACT_VERSION, 8, "the host speaks contract v8");
    assert_eq!(component_for("tinyflows"), Some("tinyflows"));
}

/// The pins are copied from the published v0.9.5 `checksum.toml`.
#[test]
fn the_flows_record_pins_every_host_of_one_release() {
    let record = find("tinyflows").unwrap();
    assert_eq!(record.version, "0.9.5");
    assert_eq!(record.assets.len(), 11);
    let expected = [
        (
            "tinyflows-module-0.9.5-macos-26-arm64.tar.gz",
            "c6e31c3c8654c28b8d0b6dfe91ca1971a00f19dae14bf9903242e769225ef9d1",
        ),
        (
            "tinyflows-module-0.9.5-macos-26-x86_64.tar.gz",
            "6531b27fddc55f8febee2113086b80ad12227aad4beeb48e0f47143da24271ed",
        ),
        (
            "tinyflows-module-0.9.5-macos-15-arm64.tar.gz",
            "2b98c6b44ed84a430d071592805f05891b1475653df4dacd96ff3f524a29a99f",
        ),
        (
            "tinyflows-module-0.9.5-macos-15-x86_64.tar.gz",
            "d5fcafcaa1885276921d26a0e65fa0d60991ea7964a7007641de21b555dfb343",
        ),
        (
            "tinyflows-module-0.9.5-ubuntu-24.04-x86_64.tar.gz",
            "f7c5e7438c64ce76c0249f81a75669b25f430250abfeff1d6831a782a2539a8b",
        ),
        (
            "tinyflows-module-0.9.5-ubuntu-24.04-arm64.tar.gz",
            "55779f03a89cd3c5d76bd5bb26e6d9837b2f5e3153c83f3aa11656222a4f4582",
        ),
        (
            "tinyflows-module-0.9.5-ubuntu-22.04-x86_64.tar.gz",
            "e6f96edfbab60421e3dc6db5d51fb49fb0da36d049cc359a5db5091b3b9861b6",
        ),
        (
            "tinyflows-module-0.9.5-ubuntu-22.04-arm64.tar.gz",
            "f09c8f4104cd17d00c5eb5f577c8048ba7b76a985e894146df5cc6b552b3e97a",
        ),
        (
            "tinyflows-module-0.9.5-windows-2025-x86_64.zip",
            "6221186d10e28a56ab8d5881e8d525f0db70cc4ccb0d5681f5893b11c7950920",
        ),
        (
            "tinyflows-module-0.9.5-windows-2022-x86_64.zip",
            "b8eec6404312c71286650132a41d33758af2efa49c611a131befd383b1c77aca",
        ),
        (
            "tinyflows-module-0.9.5-windows-11-arm64.zip",
            "67d4b8c8ad87c4a7c416ddaff4c96c79bde0fc2e43624306b219e85c8c332587",
        ),
    ];
    for (asset, (archive, sha256)) in record.assets.iter().zip(expected) {
        assert_eq!(asset.archive, archive);
        assert_eq!(asset.sha256, sha256, "{}", asset.archive);
    }
}
