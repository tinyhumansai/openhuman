//! Provision a core through OpenHuman's typed TinyBox module facade.
//!
//! The host chooses the placement, workspace, network policy, and core
//! credentials. TinyBox owns every native sandbox, process, and forward handle.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::Duration;

use openhuman_rpc::embed::config::RuntimeConfig;
use openhuman_rpc::embed::modules::tinybox::{self, contract};

use super::ops::{ProgressSink, Provisioned};
use super::types::{ActiveGateway, Confinement, Gateway, Reach, CORE_PORT_IN_BOX};

/// How long to wait for a provisioned core to answer `/health`.
const HEALTH_TIMEOUT: Duration = Duration::from_secs(120);
/// How often to re-ask while waiting.
const HEALTH_POLL: Duration = Duration::from_millis(250);

/// Create a box, start a core in it, and make it reachable from here.
pub(super) async fn provision(
    gateway: &Gateway,
    reach: &Reach,
    confinement: &Confinement,
    env: &BTreeMap<String, String>,
    progress: &ProgressSink,
) -> Result<Provisioned, String> {
    // This is intentionally loaded through the curated facade: gateway
    // activation happens before the embedded RPC server may exist.
    let config = openhuman_rpc::embed::config::load_config_with_timeout().await?;
    let token = crate::core_process::generate_rpc_token();

    progress("reserving the box");
    let resource = tinybox::reserve(&config, contract::ReserveRequest::Resource)
        .await
        .map_err(|error| format!("could not reserve the box: {error}"))?;

    progress("creating the box");
    let request = create_request(resource.clone(), reach, confinement, env);
    let info = match tinybox::create(&config, request).await {
        Ok(info) if info.resource == resource => info,
        Ok(_) => {
            close_quietly(&config, resource).await;
            return Err("TinyBox returned a mismatched resource reservation".to_owned());
        }
        Err(error) => {
            // The reply may have been lost after native creation. The
            // reservation is caller-known, so Close can still clean it up.
            close_quietly(&config, resource).await;
            return Err(format!("could not create the box: {error}"));
        }
    };
    log::info!(
        "[gateway][provision] TinyBox resource created for {}",
        gateway.id
    );

    progress("starting the core");
    let process_reservation = match tinybox::reserve(
        &config,
        contract::ReserveRequest::Process(info.resource.clone()),
    )
    .await
    {
        Ok(process) => process,
        Err(error) => {
            close_quietly(&config, resource).await;
            return Err(format!("could not reserve the core process: {error}"));
        }
    };
    let command = core_command(&info.resource, confinement, &token);
    let process = match tinybox::spawn(
        &config,
        contract::SpawnRequest {
            process: process_reservation,
            command,
        },
    )
    .await
    {
        Ok(process) if process.resource == info.resource => process,
        Ok(_) => {
            close_quietly(&config, resource).await;
            return Err("TinyBox returned a mismatched process resource".to_owned());
        }
        Err(error) => {
            close_quietly(&config, resource).await;
            return Err(format!("could not start the core in the box: {error}"));
        }
    };

    progress("opening the connection");
    let forward_reservation = match tinybox::reserve(
        &config,
        contract::ReserveRequest::Forward(info.resource.clone()),
    )
    .await
    {
        Ok(forward) => forward,
        Err(error) => {
            close_quietly(&config, resource).await;
            return Err(format!("could not reserve the gateway connection: {error}"));
        }
    };
    let forward = match tinybox::forward(
        &config,
        contract::ForwardRequest {
            resource: info.resource.clone(),
            forward: forward_reservation,
            guest_port: CORE_PORT_IN_BOX,
        },
    )
    .await
    {
        Ok(forward) if forward.resource == info.resource => forward,
        Ok(_) => {
            close_quietly(&config, resource).await;
            return Err("TinyBox returned a mismatched forward resource".to_owned());
        }
        Err(error) => {
            close_quietly(&config, resource).await;
            return Err(format!("could not reach the box: {error}"));
        }
    };
    let address: SocketAddr = match forward.local_address.parse() {
        Ok(address) => address,
        Err(_) => {
            close_quietly(&config, resource).await;
            return Err("TinyBox returned an invalid gateway address".to_owned());
        }
    };
    let rpc_base = format!("http://{address}");

    progress("waiting for the core");
    if let Err(error) = wait_until_healthy(&rpc_base).await {
        close_quietly(&config, resource).await;
        return Err(error);
    }

    log::info!(
        "[gateway][provision] {} ready at {rpc_base}/rpc",
        gateway.id
    );
    Ok(Provisioned {
        active: ActiveGateway {
            id: gateway.id.clone(),
            rpc_url: format!("{rpc_base}/rpc"),
            token: Some(token),
        },
        config,
        resource,
        process,
    })
}

/// Construct the module request without exposing TinyBox implementation types.
pub(super) fn create_request(
    resource: contract::ResourceId,
    reach: &Reach,
    confinement: &Confinement,
    env: &BTreeMap<String, String>,
) -> contract::CreateRequest {
    let (backend, workspace, network, ports) = match confinement {
        Confinement::Passthrough { workspace, .. } => (
            "passthrough",
            contract::Workspace::Directory(
                workspace
                    .clone()
                    .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                    .display()
                    .to_string(),
            ),
            contract::NetworkPolicy::Open,
            Vec::new(),
        ),
        Confinement::Docker { image } => (
            "docker",
            contract::Workspace::Image(image.clone()),
            contract::NetworkPolicy::Egress,
            vec![contract::PortMapping {
                guest: CORE_PORT_IN_BOX,
                host: None,
            }],
        ),
    };

    contract::CreateRequest {
        resource,
        backend: backend.to_owned(),
        host: host_config(reach),
        workspace,
        network,
        resources: contract::ResourceLimits::default(),
        ports,
        env: env.clone(),
    }
}

pub(super) fn host_config(reach: &Reach) -> contract::HostConfig {
    match reach {
        Reach::Local => contract::HostConfig::Local,
        Reach::Ssh(ssh) => contract::HostConfig::Ssh(contract::SshHostConfig {
            destination: ssh.destination.clone(),
            port: ssh.port,
            identity: ssh.identity.as_ref().map(|path| path.display().to_string()),
            known_hosts: None,
            accept_new_host_key: ssh.accept_new_host_key,
        }),
    }
}

/// Build the explicit core command passed to TinyBox.
pub(super) fn core_command(
    resource: &contract::ResourceId,
    confinement: &Confinement,
    token: &str,
) -> contract::ExecRequest {
    let binary = match confinement {
        Confinement::Passthrough { binary, .. } => binary.display().to_string(),
        Confinement::Docker { .. } => "openhuman-core".to_owned(),
    };
    let mut env = BTreeMap::new();
    env.insert("OPENHUMAN_CORE_TOKEN".to_owned(), token.to_owned());
    env.insert("OPENHUMAN_CORE_HOST".to_owned(), "0.0.0.0".to_owned());
    env.insert(
        "OPENHUMAN_CORE_PORT".to_owned(),
        CORE_PORT_IN_BOX.to_string(),
    );
    let cwd = match confinement {
        Confinement::Passthrough { workspace, .. } => {
            workspace.as_ref().map(|path| path.display().to_string())
        }
        Confinement::Docker { .. } => None,
    };
    contract::ExecRequest {
        resource: resource.clone(),
        argv: vec![binary, "serve".to_owned()],
        cwd,
        env,
        stdin: None,
    }
}

async fn close_quietly(config: &RuntimeConfig, resource: contract::ResourceId) {
    if let Err(error) = tinybox::close(config, resource).await {
        log::warn!("[gateway][teardown] TinyBox cleanup remains pending: {error}");
    }
}

/// Poll `/health` until the core answers, or give up.
async fn wait_until_healthy(base_url: &str) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + HEALTH_TIMEOUT;
    let client = reqwest::Client::new();
    loop {
        match client.get(format!("{base_url}/health")).send().await {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(_) | Err(_) => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("the core did not become healthy within two minutes".to_owned());
        }
        tokio::time::sleep(HEALTH_POLL).await;
    }
}
