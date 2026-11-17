//! Turning a [`GatewaySpec`] into somewhere the frontend can send RPC.
//!
//! # The whole design in one sentence
//!
//! A gateway resolves to a URL and a bearer, and nothing downstream changes.
//! `core_rpc_url` and `core_rpc_token` answer from the active gateway, so every
//! existing caller — `coreRpcClient`, `relay_http_rpc`, every screen — reaches a
//! container on another machine through exactly the code that reached the core
//! in this process.
//!
//! # What tinybox contributes
//!
//! Two axes that compose: *reach* (`local` / `ssh`) and *confinement*
//! (`passthrough` / `docker`). Pairing them costs nothing, so "a container on
//! the build server" needs no code naming that combination — which is why
//! [`GatewaySpec::Box`] is one variant rather than three.
//!
//! Getting a core into a box and making it reachable is [`provision`]'s job;
//! this module decides *which* of those a gateway needs and holds the result.

use openhuman_rpc::embed::config::RuntimeConfig;
use openhuman_rpc::embed::modules::tinybox::{self, contract};

use super::provision;
use super::types::{ActiveGateway, Gateway, GatewaySpec};

/// A provisioned gateway, and everything that has to stay alive for it.
///
/// The module-owned resource remains active while this handle is stored. The
/// registry calls [`Self::tear_down`] to await acknowledged cleanup before
/// releasing it.
pub struct Provisioned {
    /// Where the frontend should send RPC.
    pub active: ActiveGateway,
    /// The tunnel, if reaching the box needed one. Dropping it closes it.
    ///
    /// The three below are `pub(super)` only so [`provision`] can build one:
    /// it is the sibling that creates all of this, and moving it back in here
    /// would put four hundred lines of box mechanics in the module whose job is
    /// deciding which gateway needs them.
    pub(super) config: RuntimeConfig,
    /// Caller-known module resource handle used for acknowledged cleanup.
    pub(super) resource: contract::ResourceId,
    /// Opaque process id used only in safe diagnostics; TinyBox owns the process.
    pub(super) process: contract::ProcessRef,
}

impl std::fmt::Debug for Provisioned {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Hand-written because `ActiveGateway` carries a bearer, and a derived
        // `Debug` would put it in any log line that formats this.
        formatter
            .debug_struct("Provisioned")
            .field("id", &self.active.id)
            .field("resource", &self.resource.0)
            .field("process", &self.process.process.0)
            .finish_non_exhaustive()
    }
}

impl Provisioned {
    /// Close the TinyBox resource and wait for native cleanup acknowledgement.
    ///
    /// Best-effort at the gateway-switch boundary: cleanup failures are logged
    /// without blocking the user from reaching another gateway. TinyBox keeps
    /// any unacknowledged native ownership for module shutdown or retry.
    pub async fn tear_down(self) {
        log::debug!(
            "[gateway][teardown] closing TinyBox resource {}",
            self.resource.0
        );
        if let Err(error) = tinybox::close(&self.config, self.resource).await {
            log::warn!("[gateway][teardown] cleanup remains pending: {error}");
        }
    }
}

/// Progress, reported as each step completes.
///
/// Provisioning takes tens of seconds and the steps fail for different reasons,
/// so the UI shows which one is happening rather than an untimed spinner that
/// says nothing about whether an image pull is stuck.
pub type ProgressSink = dyn Fn(&str) + Send + Sync;

/// Resolve `gateway` into somewhere the frontend can send RPC.
///
/// `Desktop` and `Remote` resolve without provisioning anything, so they return
/// `None` for the second half of the pair — there is nothing to hold open and
/// nothing to tear down.
///
/// # Errors
///
/// Returns a user-facing message when the box cannot be created, the core
/// cannot be started, the tunnel cannot be opened, or the core never becomes
/// healthy. Every message is safe to display: no bearer, no key path.
pub async fn activate(
    gateway: &Gateway,
    desktop: &crate::core_process::CoreProcessHandle,
    progress: &ProgressSink,
) -> Result<Option<Provisioned>, String> {
    log::info!(
        "[gateway][activate] id={} kind={}",
        gateway.id,
        gateway.spec.kind()
    );

    match &gateway.spec {
        GatewaySpec::Desktop => {
            progress("starting the local core");
            desktop.ensure_running().await?;
            log::debug!(
                "[gateway][activate] desktop core ready on {}",
                desktop.port()
            );
            Ok(None)
        }
        GatewaySpec::Remote { url, .. } => {
            // Nothing to provision: someone else is running this core, and the
            // URL is the whole answer. Reachability is still the caller's to
            // check, exactly as it is for a provisioned one.
            log::debug!(
                "[gateway][activate] remote endpoint {}",
                crate::core_rpc::redact_url_for_log(url)
            );
            Ok(None)
        }
        GatewaySpec::Box {
            reach,
            confinement,
            env,
        } => provision::provision(gateway, reach, confinement, env, progress)
            .await
            .map(Some),
    }
}

/// The URL and bearer a non-provisioning gateway resolves to.
///
/// Split from [`activate`] because these two need no async work at all: the
/// answer is already in the record (or in the handle), so making callers await
/// it would suggest otherwise.
#[must_use]
pub fn endpoint_of(
    gateway: &Gateway,
    desktop: &crate::core_process::CoreProcessHandle,
) -> Option<ActiveGateway> {
    match &gateway.spec {
        GatewaySpec::Desktop => Some(ActiveGateway {
            id: gateway.id.clone(),
            rpc_url: desktop.rpc_url(),
            token: Some(desktop.rpc_token().to_owned()),
        }),
        GatewaySpec::Remote { url, token } => Some(ActiveGateway {
            id: gateway.id.clone(),
            rpc_url: url.clone(),
            token: token.clone(),
        }),
        GatewaySpec::Box { .. } => None,
    }
}
