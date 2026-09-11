//! Tests for the active-gateway registry.
//!
//! Provisioning a real box needs a daemon, so what is checked here is the part
//! that is true regardless: that there is always a working answer, and that a
//! failed activation leaves the previous gateway in place rather than stranding
//! the user with nothing.

use super::registry;
use super::store;
use super::types::{Confinement, Gateway, GatewaySpec, Reach, DESKTOP_ID};

/// A gateway that cannot possibly activate: the destination is in a reserved
/// TLD, so no test can reach a real machine.
fn unreachable() -> Gateway {
    Gateway {
        id: "unreachable".to_owned(),
        label: "Nowhere".to_owned(),
        spec: GatewaySpec::Box {
            reach: Reach::Ssh(super::types::SshReach {
                destination: "nobody@example.invalid".to_owned(),
                ..Default::default()
            }),
            confinement: Confinement::Docker {
                image: "openhuman-core:latest".to_owned(),
            },
            env: Default::default(),
        },
    }
}

#[tokio::test]
async fn before_anything_is_activated_rpc_goes_to_the_core_in_this_process() {
    // The state at launch, and the state after a failed activation. There is
    // always a working answer because this core is always reachable.
    let desktop = crate::core_process::CoreProcessHandle::new(7788);

    let active = registry::current(&desktop).await;

    assert_eq!(active.id, DESKTOP_ID);
    assert_eq!(active.rpc_url, desktop.rpc_url());
    assert_eq!(active.token.as_deref(), Some(desktop.rpc_token()));
}

#[tokio::test]
async fn the_desktop_gateway_is_active_by_default() {
    assert_eq!(registry::active_id().await, DESKTOP_ID);
}

#[tokio::test]
async fn a_gateway_that_is_not_active_reports_inactive() {
    let status = registry::status_of("some-other-gateway").await;

    assert_eq!(status, super::types::GatewayStatus::Inactive);
}

#[tokio::test]
async fn a_failed_activation_leaves_rpc_pointing_somewhere_that_works() {
    // The ordering that matters: tear down the old gateway only after the new
    // one is up. Otherwise a typo in an SSH destination takes the app offline.
    let desktop = crate::core_process::CoreProcessHandle::new(7788);
    let before = registry::current(&desktop).await;

    let outcome = registry::activate(&unreachable(), &desktop).await;

    assert!(outcome.is_err(), "example.invalid must not activate");
    let after = registry::current(&desktop).await;
    assert_eq!(after.rpc_url, before.rpc_url);
}

#[test]
fn a_gateway_is_looked_up_by_the_id_the_frontend_holds() {
    // The renderer stores an id, not a spec, so that an SSH identity path and a
    // remote bearer never reach renderer-accessible storage.
    let desktop = store::get(DESKTOP_ID).expect("the desktop gateway always resolves");

    assert_eq!(desktop.spec, GatewaySpec::Desktop);
}
