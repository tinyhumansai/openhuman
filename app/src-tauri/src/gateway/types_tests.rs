//! Tests for the gateway model.
//!
//! Two things matter here and neither is obvious from the type definitions.
//! The wire shape is a contract with the renderer, so it is pinned literally.
//! And `kind()` is what the picker labels a gateway with, which is the one
//! place the reach/confinement split becomes visible to a user.

use super::types::{Confinement, Gateway, GatewaySpec, Reach, SshReach, DESKTOP_ID};

fn ssh(destination: &str) -> Reach {
    Reach::Ssh(SshReach {
        destination: destination.to_owned(),
        ..SshReach::default()
    })
}

fn docker() -> Confinement {
    Confinement::Docker {
        image: "openhuman-core:latest".to_owned(),
    }
}

#[test]
fn the_two_axes_compose_without_a_variant_for_the_pairing() {
    // The point of borrowing tinybox's model: "a container on the build
    // server" is not a third case, it is the two choices made independently.
    let here = GatewaySpec::Box {
        reach: Reach::Local,
        confinement: docker(),
        env: Default::default(),
    };
    let over_there = GatewaySpec::Box {
        reach: ssh("builder@example.com"),
        confinement: docker(),
        env: Default::default(),
    };

    assert_eq!(here.kind(), "docker");
    assert_eq!(over_there.kind(), "ssh+docker");
}

#[test]
fn only_provisioned_gateways_provision() {
    // `Desktop` and `Remote` have nothing to create and nothing to tear down,
    // which is why `activate` returns no handle for them.
    assert!(!GatewaySpec::Desktop.provisions());
    assert!(!GatewaySpec::Remote {
        url: "https://core.example.com/rpc".to_owned(),
        token: None,
    }
    .provisions());
    assert!(GatewaySpec::Box {
        reach: Reach::Local,
        confinement: docker(),
        env: Default::default(),
    }
    .provisions());
}

#[test]
fn a_spec_round_trips_through_its_wire_shape() {
    // The renderer builds these, so the tag names are a contract. A rename
    // here without one there is a gateway the user can save and never activate.
    let spec = GatewaySpec::Box {
        reach: ssh("builder@example.com"),
        confinement: docker(),
        env: Default::default(),
    };

    let json = serde_json::to_value(&spec).expect("serializable");
    assert_eq!(json["kind"], "box");
    assert_eq!(json["reach"]["kind"], "ssh");
    assert_eq!(json["reach"]["destination"], "builder@example.com");
    assert_eq!(json["confinement"]["kind"], "docker");

    let back: GatewaySpec = serde_json::from_value(json).expect("deserializable");
    assert_eq!(back, spec);
}

#[test]
fn ssh_options_are_omitted_rather_than_sent_as_null() {
    // A form that leaves the optional fields blank should produce a record
    // that reads as "unset", not one that has to be distinguished from it.
    let spec = GatewaySpec::Box {
        reach: ssh("host"),
        confinement: docker(),
        env: Default::default(),
    };

    let json = serde_json::to_value(&spec).expect("serializable");
    assert!(json["reach"].get("port").is_none());
    assert!(json["reach"].get("identity").is_none());
}

#[test]
fn the_desktop_gateway_is_a_record_rather_than_a_special_case() {
    // So the picker always has something selected, and "no gateways
    // configured" is never a state the UI has to render.
    let desktop = Gateway::desktop();

    assert_eq!(desktop.id, DESKTOP_ID);
    assert_eq!(desktop.spec, GatewaySpec::Desktop);
}
