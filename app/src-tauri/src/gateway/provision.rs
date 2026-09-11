//! Making a box run a core, and making that core reachable from here.
//!
//! Four steps, each one tinybox call:
//!
//! 1. `create` a box, publishing the core's port to whichever machine it runs on
//! 2. `spawn` the core in it, detached, with a freshly minted bearer
//! 3. `forward` that published port back to this machine
//! 4. poll the core's unauthenticated `/health` until it answers
//!
//! Step 3 is the one that is easy to leave out and impossible to notice
//! missing: publishing puts the port on the *box's* host, which for an SSH
//! placement is the far machine. Everything looks configured and nothing is
//! reachable.
//!
//! Step 4 is not a courtesy either. A tunnel's local listener exists before the
//! far side is proven, so "the forward opened" is not "the core is up".

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use tinybox_core::{
    BoxId, BoxInfo, BoxSpec, ExecRequest, Host, NetworkPolicy, PassthroughSandbox, Placement,
    PortMapping, ProcessId, Sandbox, WorkspaceSource,
};
use tinybox_docker::DockerSandbox;
use tinybox_host::LocalHost;
use tinybox_ssh::{SshHost, SshTarget};

use super::ops::{ProgressSink, Provisioned};
use super::types::{ActiveGateway, Confinement, Gateway, Reach, SshReach, CORE_PORT_IN_BOX};

/// How long to wait for a provisioned core to answer `/health`.
///
/// Generous because this covers a container start, the core's own boot, and
/// possibly an image pull - and because polling returns the moment it answers,
/// so a high ceiling costs nothing when things are fast. Mirrors the embedded
/// core's own ceiling in `core_process`.
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
    let host = build_host(reach)?;
    let sandbox = build_sandbox(confinement, &host);

    // Minted here and handed to the core as an environment variable, so it is
    // never written to disk and never reused across activations. The core
    // reads `OPENHUMAN_CORE_TOKEN` and gates `/rpc` on it; `/health` stays
    // unauthenticated, which is what makes the readiness poll below possible
    // before any credential is established.
    let token = crate::core_process::generate_rpc_token();

    progress("creating the box");
    let (info, host_port) = create_box(sandbox.as_ref(), reach, confinement, env).await?;
    log::info!(
        "[gateway][provision] box {} created ({}) publishing {host_port}",
        info.id,
        sandbox.name()
    );

    progress("starting the core");
    let started = start_core(sandbox.as_ref(), &info.id, confinement, &token).await;
    let process = match started {
        Ok(process) => process,
        Err(error) => {
            // Do not leave a box behind for a core that never started.
            destroy_quietly(sandbox.as_ref(), &info.id).await;
            return Err(error);
        }
    };

    progress("opening the connection");
    let forwarded = host
        .forward(([127, 0, 0, 1], host_port).into())
        .await
        .map_err(|error| format!("could not reach the box: {error}"));
    let forwarded = match forwarded {
        Ok(forwarded) => forwarded,
        Err(error) => {
            stop_quietly(sandbox.as_ref(), &info.id, &process).await;
            destroy_quietly(sandbox.as_ref(), &info.id).await;
            return Err(error);
        }
    };

    let rpc_base = format!("http://{}", forwarded.local_addr());
    progress("waiting for the core");
    if let Err(error) = wait_until_healthy(&rpc_base).await {
        stop_quietly(sandbox.as_ref(), &info.id, &process).await;
        destroy_quietly(sandbox.as_ref(), &info.id).await;
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
        _forward: Some(forwarded),
        sandbox,
        box_id: info.id,
        process,
    })
}

/// The host a box's commands run on.
fn build_host(reach: &Reach) -> Result<Arc<dyn Host>, String> {
    match reach {
        Reach::Local => Ok(Arc::new(LocalHost::new())),
        Reach::Ssh(ssh) => Ok(Arc::new(SshHost::new(
            // Always local: `SshHost` opens its tunnel from its inner host, and
            // a chained one would open it on the wrong machine. tinybox refuses
            // that case rather than reporting an address leading nowhere.
            Arc::new(LocalHost::new()),
            ssh_target(ssh)?,
        ))),
    }
}

fn ssh_target(ssh: &SshReach) -> Result<SshTarget, String> {
    let mut target = SshTarget::new(ssh.destination.clone())
        .map_err(|error| format!("that SSH destination is not usable: {error}"))?;
    if let Some(port) = ssh.port {
        target = target.with_port(port);
    }
    if let Some(identity) = &ssh.identity {
        target = target.with_identity(identity.clone());
    }
    if ssh.accept_new_host_key {
        target = target.accepting_new_host_key();
    }
    Ok(target)
}

/// The sandbox a box is created in.
fn build_sandbox(confinement: &Confinement, host: &Arc<dyn Host>) -> Arc<dyn Sandbox> {
    // An in-memory store: these boxes exist for as long as this process holds
    // the gateway open, and a file store would outlive that — leaving records
    // for containers a later run has no tunnel to and no reason to trust.
    let store = Arc::new(tinybox_core::MemoryStore::new());
    match confinement {
        Confinement::Passthrough { .. } => {
            Arc::new(PassthroughSandbox::new(host.clone(), store)) as Arc<dyn Sandbox>
        }
        Confinement::Docker { .. } => {
            Arc::new(DockerSandbox::new(host.clone(), store)) as Arc<dyn Sandbox>
        }
    }
}

/// The spec the box is created from.
pub(super) fn box_spec(
    reach: &Reach,
    confinement: &Confinement,
    env: &BTreeMap<String, String>,
    host_port: u16,
) -> Result<BoxSpec, String> {
    let host_ref = match reach {
        Reach::Local => "local",
        Reach::Ssh(_) => "ssh",
    };
    let (sandbox_ref, source) = match confinement {
        Confinement::Passthrough { workspace, .. } => (
            "passthrough",
            WorkspaceSource::LocalDir(
                workspace
                    .clone()
                    .unwrap_or_else(|| std::path::PathBuf::from("/tmp")),
            ),
        ),
        Confinement::Docker { image } => ("docker", WorkspaceSource::OciImage(image.clone())),
    };

    let placement = Placement::new(
        tinybox_core::HostRef::new(host_ref)
            .map_err(|error| format!("invalid host reference: {error}"))?,
        tinybox_core::SandboxRef::new(sandbox_ref)
            .map_err(|error| format!("invalid sandbox reference: {error}"))?,
    );

    let mut spec = BoxSpec::new(placement, source);
    for (key, value) in env {
        spec = spec.with_env(key, value);
    }

    if matches!(confinement, Confinement::Docker { .. }) {
        // Publishing is how the core is reached at all, and tinybox drops the
        // `--publish` flags entirely on a box whose network is denied — a
        // container with no network has nowhere for a published port to lead.
        // `Egress` is the weakest policy that still publishes; the core needs
        // outbound anyway to reach the TinyHumans backend.
        spec = spec.with_network(NetworkPolicy::Egress);
        // `fixed(guest, host)`, in that order: the core listens on
        // CORE_PORT_IN_BOX *inside* the box, and that is what gets published
        // at `host_port` on the machine the box runs on. Reversed, the box
        // would publish a port nothing is listening on.
        spec = spec.with_port(PortMapping::fixed(CORE_PORT_IN_BOX, host_port));
    }
    Ok(spec)
}

/// The command that starts `openhuman-core` in a box.
///
/// Pure and separate from running it, for the reason tinybox's own backends
/// keep command construction in an `args` module: which binary is named and
/// what environment it is handed are the decisions worth asserting, and they
/// should be values rather than something only observable by starting a
/// container.
pub(super) fn core_command(confinement: &Confinement, token: &str) -> ExecRequest {
    let binary = match confinement {
        Confinement::Passthrough { binary, .. } => binary.display().to_string(),
        // The image's own core, on `PATH`. Naming a path here would tie the
        // gateway to one image's layout.
        Confinement::Docker { .. } => "openhuman-core".to_owned(),
    };

    ExecRequest::new([binary.as_str(), "serve"])
        // Handed over as environment and never written down: minted per
        // activation, so a stored gateway record cannot leak a credential for
        // a core that is still running.
        .with_env("OPENHUMAN_CORE_TOKEN", token)
        // Bind every interface *inside the box*, so the published port has
        // something to reach. Loopback there would be reachable only from
        // inside the container, which is the one place nothing is asking.
        .with_env("OPENHUMAN_CORE_HOST", "0.0.0.0")
        .with_env("OPENHUMAN_CORE_PORT", CORE_PORT_IN_BOX.to_string())
}

/// Start `openhuman-core` in the box, detached, and return its handle.
async fn start_core(
    sandbox: &dyn Sandbox,
    box_id: &BoxId,
    confinement: &Confinement,
    token: &str,
) -> Result<ProcessId, String> {
    sandbox
        .spawn(box_id, &core_command(confinement, token))
        .await
        .map_err(|error| format!("could not start the core in the box: {error}"))
}

/// How many times to retry creation when the chosen host port is taken.
const PORT_ATTEMPTS: usize = 8;

/// Create the box, choosing a port on its machine that is actually free.
///
/// # Why the port is named rather than left to Docker
///
/// `PortMapping::dynamic` lets Docker pick, which is the better choice in
/// general — and unusable here, because the number it picks is recorded only in
/// Docker's own state and tinybox has no call that reports it back. A forward
/// needs that number.
///
/// So the port is named, which means it can collide. On the local machine that
/// is checkable; on a remote one it is not — asking would mean running a probe
/// over there, and there is no command every host is guaranteed to have. Both
/// cases are therefore handled the same way: pick, try, and pick again if the
/// backend says the port is taken. A wrong guess is a fast, specific failure
/// from Docker rather than something silent.
async fn create_box(
    sandbox: &dyn Sandbox,
    reach: &Reach,
    confinement: &Confinement,
    env: &BTreeMap<String, String>,
) -> Result<(BoxInfo, u16), String> {
    // A passthrough box has no boundary to publish across: the core listens on
    // the machine's own port, so there is nothing to choose and nothing that
    // could collide beyond the core itself.
    if matches!(confinement, Confinement::Passthrough { .. }) {
        let spec = box_spec(reach, confinement, env, CORE_PORT_IN_BOX)?;
        let info = sandbox
            .create(&spec)
            .await
            .map_err(|error| format!("could not create the box: {error}"))?;
        return Ok((info, CORE_PORT_IN_BOX));
    }

    let mut last = String::new();
    for attempt in 1..=PORT_ATTEMPTS {
        let host_port = candidate_port(reach);
        let spec = box_spec(reach, confinement, env, host_port)?;
        match sandbox.create(&spec).await {
            Ok(info) => return Ok((info, host_port)),
            Err(error) => {
                last = error.to_string();
                if !is_port_conflict(&last) {
                    return Err(format!("could not create the box: {last}"));
                }
                log::debug!(
                    "[gateway][provision] port {host_port} is taken (attempt {attempt}/{PORT_ATTEMPTS})"
                );
            }
        }
    }
    Err(format!(
        "could not find a free port on the box's machine after {PORT_ATTEMPTS} attempts ({last})"
    ))
}

/// A port to try publishing on.
///
/// Locally the operating system is asked, which makes a collision very
/// unlikely. Remotely there is nothing to ask, so a port is drawn from the
/// ephemeral range and [`create_box`] retries if it was taken.
fn candidate_port(reach: &Reach) -> u16 {
    if matches!(reach, Reach::Local) {
        if let Some(port) = free_local_port() {
            return port;
        }
    }
    // Draw from the ephemeral range rather than counting up from a fixed base:
    // two OpenHuman instances against the same remote machine would otherwise
    // collide on their first attempt every time, and again on their second.
    use rand::Rng as _;
    rand::rng().random_range(49_152..=65_535)
}

/// A free port on this machine, or `None` if one cannot be reserved.
fn free_local_port() -> Option<u16> {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .ok()
}

/// Whether a backend's refusal was about the port rather than anything else.
///
/// Matched on text because that is what the backend gives us: tinybox passes
/// Docker's own diagnostic through verbatim, deliberately, since it is more
/// specific than anything tinybox could reconstruct. A miss here costs a retry
/// that fails the same way, not a wrong outcome — the next attempt surfaces
/// whatever the real error was.
pub(super) fn is_port_conflict(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase();
    lowered.contains("port is already allocated")
        || lowered.contains("address already in use")
        || (lowered.contains("bind") && lowered.contains("already"))
}

/// Poll `/health` until the core answers, or give up.
///
/// `/health` is unauthenticated by design, which is what makes this possible
/// before the bearer matters — and it is a genuinely necessary step rather than
/// a courtesy: a tunnel's local listener exists before the far side is proven,
/// so "the forward opened" is not "the core is up".
async fn wait_until_healthy(base_url: &str) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| format!("could not build an HTTP client: {error}"))?;
    let url = format!("{base_url}/health");
    let deadline = tokio::time::Instant::now() + HEALTH_TIMEOUT;

    loop {
        // Kept so the timeout message can name why it never answered.
        // "did not become reachable" alone sends an operator looking at the
        // network when the core was answering 503 the whole time.
        let detail = match client.get(&url).send().await {
            Ok(response) if response.status().is_success() => {
                log::debug!("[gateway][health] {base_url} is up");
                return Ok(());
            }
            Ok(response) => format!("HTTP {}", response.status()),
            Err(error) => error.to_string(),
        };

        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "the core did not become reachable within {}s ({detail})",
                HEALTH_TIMEOUT.as_secs()
            ));
        }
        tokio::time::sleep(HEALTH_POLL).await;
    }
}

/// Destroy a box, logging rather than propagating a failure.
///
/// Used on the failure paths, where the error the caller is about to see is the
/// one that matters — a cleanup failure on top of it would bury the cause.
async fn destroy_quietly(sandbox: &dyn Sandbox, box_id: &BoxId) {
    if let Err(error) = sandbox.destroy(box_id).await {
        log::warn!("[gateway][provision] could not clean up box {box_id}: {error}");
    }
}

/// Stop a started core process, logging rather than propagating a failure.
///
/// `Sandbox::destroy` on a passthrough box only removes the box record — it
/// does not stop the detached `ProcessId` — so on the forward and health-error
/// paths the core must be stopped explicitly or it keeps running headless.
async fn stop_quietly(sandbox: &dyn Sandbox, box_id: &BoxId, process: &ProcessId) {
    if let Err(error) = sandbox.stop(box_id, process).await {
        log::warn!("[gateway][provision] could not stop core {process} in box {box_id}: {error}");
    }
}
