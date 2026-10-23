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
    #[cfg(feature = "security-module")]
    names.push((
        "tinysecurity",
        tinysecurity_bus::names::INTERFACE,
        tinysecurity_bus::names::OBJECT_PATH,
    ));
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
    feature = "tinymemes",
    feature = "inbound-queues"
))]
#[test]
fn a_product_feature_build_carries_all_fifteen_records() {
    assert_eq!(ALL.len(), 15);
    for id in [
        "tinycomputer",
        "tinysecurity",
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
        "tinymemes",
        "tinyeventqueues",
    ] {
        assert!(find(id).is_some(), "{id} missing from the registry");
    }
}

#[test]
fn every_record_maps_to_a_real_component() {
    for record in ALL {
        if record.id == "tinysecurity" {
            assert!(
                component_for(record.id).is_none(),
                "security enforcement must not be gated by a product component"
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
                    "tinydocs"
                        | "tinywallet"
                        | "tinyvoice"
                        | "tinyhosts"
                        | "tinymemes"
                        | "tinyeventqueues"
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
    assert_eq!(schedule::CONTRACT_VERSION, 7, "the host speaks contract v7");
    assert_eq!(component_for("tinyflows"), Some("tinyflows"));
}

/// The pins are copied from the published v0.8.29 `checksum.toml`.
#[test]
fn the_flows_record_pins_every_host_of_one_release() {
    let record = find("tinyflows").unwrap();
    assert_eq!(record.version, "0.8.29");
    assert_eq!(record.assets.len(), 11);
    let expected = [
        (
            "tinyflows-module-0.8.29-macos-26-arm64.tar.gz",
            "af1b942b4a23f764319d3378c23b3244844026f6832d7384a57a4adfc3879ca6",
        ),
        (
            "tinyflows-module-0.8.29-macos-26-x86_64.tar.gz",
            "27916a9872608ea363421b8d4ff807c669203ee5e6546f04e33d507e2c5feb33",
        ),
        (
            "tinyflows-module-0.8.29-macos-15-arm64.tar.gz",
            "997d5aaf21340362059edbe5f715d76c10df92823dd4c93cdfea8ad6d53c21ed",
        ),
        (
            "tinyflows-module-0.8.29-macos-15-x86_64.tar.gz",
            "b2df10c5bf1f302eedf0466eebc44f92ef23c8fa8037b34183499b34b26ce614",
        ),
        (
            "tinyflows-module-0.8.29-ubuntu-24.04-x86_64.tar.gz",
            "1c61b3925b366a0a9e610549249944901f90e3191be5639a9f68663af4eb4074",
        ),
        (
            "tinyflows-module-0.8.29-ubuntu-24.04-arm64.tar.gz",
            "2ed13d6aa3e06a7bce361af9bd8ab0a2e4b2cf01d0c94fb98efd6f8a47f394c8",
        ),
        (
            "tinyflows-module-0.8.29-ubuntu-22.04-x86_64.tar.gz",
            "d607a01766039b99e3abdd9801ee3ea41991445dd828f3a1269d982268c7a1bb",
        ),
        (
            "tinyflows-module-0.8.29-ubuntu-22.04-arm64.tar.gz",
            "3c67ea6617b84e8b53ee7cf5dbe91cd133e5d5af6196625ab8b7db46d431ce6a",
        ),
        (
            "tinyflows-module-0.8.29-windows-2025-x86_64.zip",
            "e26a33311840b9fbd3cbc0e2e5525a134b96a16970d14ed14f187d7110e90589",
        ),
        (
            "tinyflows-module-0.8.29-windows-2022-x86_64.zip",
            "5f8734a5c4eeed7ab54400b896dd1f8318cc4d647954c2808b609af7b2181020",
        ),
        (
            "tinyflows-module-0.8.29-windows-11-arm64.zip",
            "ef9b65f044489cdca520eac70104cb3f3993d2df453c47906cc9b5a1c588f371",
        ),
    ];
    for (asset, (archive, sha256)) in record.assets.iter().zip(expected) {
        assert_eq!(asset.archive, archive);
        assert_eq!(asset.sha256, sha256, "{}", asset.archive);
    }
}
