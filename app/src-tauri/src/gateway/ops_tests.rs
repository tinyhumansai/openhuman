//! Tests for gateway provisioning.
//!
//! The decisions worth pinning are the ones made *before* anything runs: which
//! placement a spec names, whether the network policy actually permits the
//! publish it asks for, and what the core is started with. Those are pure, so
//! they are asserted as values rather than observed by creating a container.
//!
//! Provisioning end to end needs a Docker daemon and is `#[ignore]`d below.

use std::collections::BTreeMap;

use super::ops::endpoint_of;
use super::provision::{box_spec, core_command, is_port_conflict};
use super::types::{Confinement, Reach, SshReach, CORE_PORT_IN_BOX};

fn docker() -> Confinement {
    Confinement::Docker {
        image: "openhuman-core:latest".to_owned(),
    }
}

fn passthrough() -> Confinement {
    Confinement::Passthrough {
        binary: "/usr/local/bin/openhuman-core".into(),
        workspace: Some("/srv/openhuman".into()),
    }
}

fn ssh() -> Reach {
    Reach::Ssh(SshReach {
        destination: "builder@example.com".to_owned(),
        port: Some(2222),
        identity: None,
        accept_new_host_key: true,
    })
}

#[test]
fn a_docker_box_permits_the_network_its_published_port_needs() {
    // tinybox drops the `--publish` flags entirely on a box whose network is
    // denied — a container with no network has nowhere for a published port to
    // lead. Denied here would produce a box that looks configured, publishes
    // nothing, and is unreachable with no error anywhere.
    let spec = box_spec(&Reach::Local, &docker(), &BTreeMap::new(), 54321).expect("a spec");

    assert!(spec.network.allows_egress());
    let published = spec
        .ports
        .iter()
        .find(|mapping| mapping.guest == CORE_PORT_IN_BOX);
    assert_eq!(published.and_then(|mapping| mapping.host), Some(54321));
}

#[test]
fn the_host_port_is_named_rather_than_left_to_docker() {
    // `PortMapping::dynamic` would let Docker choose, and the number it chose
    // would live only in Docker's own state — tinybox has no call that reports
    // it back, and a forward needs that number.
    let spec = box_spec(&Reach::Local, &docker(), &BTreeMap::new(), 54321).expect("a spec");

    assert!(spec.ports.iter().all(|mapping| mapping.host.is_some()));
}

#[test]
fn a_passthrough_box_publishes_nothing_because_there_is_no_boundary() {
    // The core listens on the machine's own port; there is nothing to publish
    // across and nothing that could collide beyond the core itself.
    let spec = box_spec(&Reach::Local, &passthrough(), &BTreeMap::new(), 54321).expect("a spec");

    assert!(spec.ports.is_empty());
}

#[test]
fn the_placement_records_both_axes_independently() {
    // Which is what makes "a container on the build server" need no code of
    // its own: it is these two fields, chosen separately.
    let spec = box_spec(&ssh(), &docker(), &BTreeMap::new(), 54321).expect("a spec");

    assert_eq!(spec.workspace.host.as_str(), "ssh");
    assert_eq!(spec.workspace.sandbox.as_str(), "docker");
}

#[test]
fn a_passthrough_box_runs_in_the_configured_workspace() {
    let spec = box_spec(&Reach::Local, &passthrough(), &BTreeMap::new(), 1).expect("a spec");

    assert_eq!(
        spec.source,
        tinybox_core::WorkspaceSource::LocalDir("/srv/openhuman".into())
    );
}

#[test]
fn configured_environment_reaches_the_box() {
    let mut env = BTreeMap::new();
    env.insert(
        "BACKEND_URL".to_owned(),
        "https://api.example.com".to_owned(),
    );

    let spec = box_spec(&Reach::Local, &docker(), &env, 1).expect("a spec");

    assert_eq!(
        spec.env.get("BACKEND_URL").map(String::as_str),
        Some("https://api.example.com")
    );
}

#[test]
fn the_core_is_started_bound_to_every_interface_inside_the_box() {
    // Loopback inside a container is reachable only from inside it, which is
    // the one place nothing is asking. The published port would lead nowhere.
    let request = core_command(&docker(), "deadbeef");

    assert_eq!(
        request.env.get("OPENHUMAN_CORE_HOST").map(String::as_str),
        Some("0.0.0.0")
    );
    assert_eq!(
        request.env.get("OPENHUMAN_CORE_PORT").map(String::as_str),
        Some(CORE_PORT_IN_BOX.to_string().as_str())
    );
}

#[test]
fn the_bearer_is_handed_over_as_environment_rather_than_written_down() {
    // It is minted per activation and never persisted, so a stored gateway
    // record cannot leak a credential for a core that is still running.
    let request = core_command(&docker(), "deadbeef");

    assert_eq!(
        request.env.get("OPENHUMAN_CORE_TOKEN").map(String::as_str),
        Some("deadbeef")
    );
}

#[test]
fn a_docker_box_runs_the_image_s_own_core_rather_than_a_named_path() {
    // Naming a path would tie the gateway to one image's layout.
    let request = core_command(&docker(), "t");

    assert_eq!(request.program(), Some("openhuman-core"));
    assert_eq!(request.argv.get(1).map(String::as_str), Some("serve"));
}

#[test]
fn a_passthrough_box_runs_the_binary_the_user_named() {
    let request = core_command(&passthrough(), "t");

    assert_eq!(request.program(), Some("/usr/local/bin/openhuman-core"));
}

#[test]
fn a_taken_port_is_recognised_from_the_backend_s_own_words() {
    // tinybox passes Docker's diagnostic through verbatim, so this is what a
    // collision actually looks like coming back.
    assert!(is_port_conflict(
        "driver failed programming external connectivity on endpoint: \
         Bind for 0.0.0.0:54321 failed: port is already allocated"
    ));
    assert!(is_port_conflict("address already in use"));
}

#[test]
fn an_unrelated_failure_is_not_mistaken_for_a_taken_port() {
    // Retrying a missing image seven more times would turn one clear error
    // into a slow, confusing one.
    assert!(!is_port_conflict(
        "Unable to find image 'openhuman-core:latest' locally"
    ));
    assert!(!is_port_conflict("Cannot connect to the Docker daemon"));
}

#[test]
fn a_remote_gateway_resolves_to_its_url_without_provisioning() {
    use super::types::{Gateway, GatewaySpec};

    let gateway = Gateway {
        id: "cloud".to_owned(),
        label: "Cloud".to_owned(),
        spec: GatewaySpec::Remote {
            url: "https://core.example.com/rpc".to_owned(),
            token: Some("bearer".to_owned()),
        },
    };

    // The handle is irrelevant for a remote gateway and is never consulted;
    // it is required only because the desktop arm of the same function needs
    // one.
    let unused = crate::core_process::CoreProcessHandle::new(7788);
    let resolved = endpoint_of(&gateway, &unused).expect("an endpoint");

    assert_eq!(resolved.rpc_url, "https://core.example.com/rpc");
    assert_eq!(resolved.token.as_deref(), Some("bearer"));
}

/// End-to-end provisioning against a real box, with a stand-in for the core.
///
/// Everything the gateway itself owns is exercised for real here: a box is
/// created, a long-lived process is started in it and outlives the call that
/// started it, the host is asked for reach, and the endpoint is polled until it
/// answers. Only the program is substituted — a shell script that serves
/// `/health`, because building `openhuman-core` to assert that a *gateway*
/// works would test the wrong thing and take half an hour.
///
/// `#[ignore]` because it spawns processes and binds a port. Run with:
/// `cargo test --manifest-path app/src-tauri/Cargo.toml --lib
///  gateway::ops_tests::provisioning -- --ignored --nocapture`
mod provisioning {
    use std::collections::BTreeMap;

    use super::super::ops;
    use super::super::types::{Confinement, Gateway, GatewaySpec, Reach, CORE_PORT_IN_BOX};

    /// A stand-in core: answers `/health` on the port it is handed, forever.
    ///
    /// Written to disk rather than shipped as a fixture so the test is
    /// self-contained, and in `sh` because that is the one interpreter every
    /// box tinybox can host a server in already has.
    fn fake_core(dir: &std::path::Path) -> std::path::PathBuf {
        let path = dir.join("fake-openhuman-core");
        std::fs::write(
            &path,
            "#!/bin/sh\n\
             # Ignores argv (`serve`) and reads the same environment the real\n\
             # core does, so the gateway's own contract is what is under test.\n\
             while true; do\n\
             printf 'HTTP/1.1 200 OK\\r\\nContent-Length: 2\\r\\n\\r\\nok' \\\n\
             | nc -l -p \"${OPENHUMAN_CORE_PORT}\" >/dev/null 2>&1 || sleep 1\n\
             done\n",
        )
        .expect("write the stand-in core");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("make it executable");
        }
        path
    }

    #[tokio::test]
    #[ignore = "spawns a process and binds a port; run explicitly"]
    async fn a_local_box_is_provisioned_and_answers_through_its_endpoint() {
        let dir = tempfile::TempDir::new().expect("a temporary directory");
        let gateway = Gateway {
            id: "e2e".to_owned(),
            label: "End to end".to_owned(),
            spec: GatewaySpec::Box {
                reach: Reach::Local,
                confinement: Confinement::Passthrough {
                    binary: fake_core(dir.path()),
                    workspace: Some(dir.path().to_path_buf()),
                },
                env: BTreeMap::new(),
            },
        };
        let desktop = crate::core_process::CoreProcessHandle::new(0);
        let progress = |step: &str| println!("[e2e] {step}");

        let provisioned = ops::activate(&gateway, &desktop, &progress)
            .await
            .expect("provisioning succeeds")
            .expect("a box gateway is provisioned");

        // The endpoint is what the frontend would be handed, and it answers —
        // which is the whole claim this feature makes.
        assert!(provisioned.active.rpc_url.ends_with("/rpc"));
        assert!(
            provisioned.active.token.is_some(),
            "a provisioned core is behind a bearer"
        );
        assert!(
            provisioned
                .active
                .rpc_url
                .contains(&CORE_PORT_IN_BOX.to_string()),
            "a passthrough box publishes nothing, so the core's own port is the endpoint: {}",
            provisioned.active.rpc_url
        );

        provisioned.tear_down().await;
    }
}
