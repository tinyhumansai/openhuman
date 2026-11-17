//! Tests for the host-owned TinyBox module request mapping.

use std::collections::BTreeMap;

use super::ops::endpoint_of;
use super::provision::{core_command, create_request, host_config};
use super::types::{Confinement, Reach, SshReach, CORE_PORT_IN_BOX};
use openhuman_rpc::embed::modules::tinybox::contract;

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
        identity: Some("/home/operator/.ssh/build_key".into()),
        accept_new_host_key: true,
    })
}

#[test]
fn docker_request_uses_module_owned_dynamic_port_and_required_egress() {
    let request = create_request(
        contract::ResourceId("resource-1".to_owned()),
        &Reach::Local,
        &docker(),
        &BTreeMap::new(),
    );

    assert_eq!(request.backend, "docker");
    assert_eq!(request.host, contract::HostConfig::Local);
    assert_eq!(
        request.workspace,
        contract::Workspace::Image("openhuman-core:latest".to_owned())
    );
    assert_eq!(request.network, contract::NetworkPolicy::Egress);
    assert_eq!(
        request.ports,
        [contract::PortMapping {
            guest: CORE_PORT_IN_BOX,
            host: None
        }]
    );
}

#[test]
fn passthrough_request_uses_the_configured_workspace_without_publishing() {
    let request = create_request(
        contract::ResourceId("resource-1".to_owned()),
        &Reach::Local,
        &passthrough(),
        &BTreeMap::new(),
    );

    assert_eq!(request.backend, "passthrough");
    assert_eq!(
        request.workspace,
        contract::Workspace::Directory("/srv/openhuman".to_owned())
    );
    assert!(request.ports.is_empty());
}

#[test]
fn ssh_placement_and_identity_path_are_transferred_as_host_configuration() {
    assert_eq!(
        host_config(&ssh()),
        contract::HostConfig::Ssh(contract::SshHostConfig {
            destination: "builder@example.com".to_owned(),
            port: Some(2222),
            identity: Some("/home/operator/.ssh/build_key".to_owned()),
            known_hosts: None,
            accept_new_host_key: true,
        })
    );
}

#[test]
fn configured_environment_is_passed_to_module_creation() {
    let env = BTreeMap::from([(
        "BACKEND_URL".to_owned(),
        "https://api.example.com".to_owned(),
    )]);
    let request = create_request(
        contract::ResourceId("resource-1".to_owned()),
        &Reach::Local,
        &docker(),
        &env,
    );
    assert_eq!(request.env, env);
}

#[test]
fn core_command_carries_ephemeral_bearer_and_explicit_bind_address() {
    let request = core_command(
        &contract::ResourceId("resource-1".to_owned()),
        &docker(),
        "deadbeef",
    );
    assert_eq!(request.argv, ["openhuman-core", "serve"]);
    assert_eq!(
        request.env.get("OPENHUMAN_CORE_TOKEN").map(String::as_str),
        Some("deadbeef")
    );
    assert_eq!(
        request.env.get("OPENHUMAN_CORE_HOST").map(String::as_str),
        Some("0.0.0.0")
    );
    assert_eq!(
        request.env.get("OPENHUMAN_CORE_PORT").map(String::as_str),
        Some("7788")
    );
}

#[test]
fn passthrough_core_command_uses_named_binary_and_workspace() {
    let request = core_command(
        &contract::ResourceId("resource-1".to_owned()),
        &passthrough(),
        "t",
    );
    assert_eq!(request.argv[0], "/usr/local/bin/openhuman-core");
    assert_eq!(request.cwd.as_deref(), Some("/srv/openhuman"));
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
/// `cargo test --manifest-path crates/openhuman-app/Cargo.toml --lib
///  gateway::ops_tests::provisioning -- --ignored --nocapture`
mod provisioning {
    use std::collections::BTreeMap;

    use super::super::ops;
    use super::super::types::{Confinement, Gateway, GatewaySpec, Reach};

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
    #[ignore = "spawns the pinned TinyBox module plus a netcat-served stand-in core on a real port (needs `nc` with -l -p); run: cargo test --manifest-path crates/openhuman-app/Cargo.toml --lib gateway::ops_tests::provisioning -- --ignored --nocapture"]
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
        let endpoint: url::Url = provisioned
            .active
            .rpc_url
            .parse()
            .expect("TinyBox returns a parseable local forwarding address");
        assert!(
            endpoint.port().is_some(),
            "forwarded address has an assigned port"
        );

        provisioned.tear_down().await;
    }
}
