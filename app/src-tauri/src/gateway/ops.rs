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

use std::sync::Arc;

use tinybox_core::{BoxId, Forward, ProcessId, Sandbox};

use super::provision;
use super::types::{ActiveGateway, Gateway, GatewaySpec};

/// A provisioned gateway, and everything that has to stay alive for it.
///
/// Holding this *is* the gateway existing: dropping it closes the tunnel and
/// the box stops being reachable. That is why the active one is kept in a
/// long-lived registry rather than returned to the caller.
pub struct Provisioned {
    /// Where the frontend should send RPC.
    pub active: ActiveGateway,
    /// The tunnel, if reaching the box needed one. Dropping it closes it.
    ///
    /// The three below are `pub(super)` only so [`provision`] can build one:
    /// it is the sibling that creates all of this, and moving it back in here
    /// would put four hundred lines of box mechanics in the module whose job is
    /// deciding which gateway needs them.
    pub(super) _forward: Option<Forward>,
    /// The sandbox the box lives in, for tearing it down.
    pub(super) sandbox: Arc<dyn Sandbox>,
    /// The box.
    pub(super) box_id: BoxId,
    /// The core process inside it.
    pub(super) process: ProcessId,
}

impl std::fmt::Debug for Provisioned {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Hand-written because `ActiveGateway` carries a bearer, and a derived
        // `Debug` would put it in any log line that formats this.
        formatter
            .debug_struct("Provisioned")
            .field("id", &self.active.id)
            .field("box", &self.box_id.as_str())
            .field("process", &self.process.as_str())
            .finish_non_exhaustive()
    }
}

impl Provisioned {
    /// Stop the core and destroy the box.
    ///
    /// Best-effort throughout: this runs when the user is switching away, and
    /// a box that cannot be cleaned up must not block them from reaching a
    /// gateway that works. Every failure is logged and the next step is still
    /// attempted.
    pub async fn tear_down(self) {
        log::debug!(
            "[gateway][teardown] stopping {} in box {}",
            self.process,
            self.box_id
        );
        if let Err(error) = self.sandbox.stop(&self.box_id, &self.process).await {
            log::warn!("[gateway][teardown] stop failed: {error}");
        }
        if let Err(error) = self.sandbox.destroy(&self.box_id).await {
            log::warn!("[gateway][teardown] destroy failed: {error}");
        }
        // The forward's own `Drop` closes the tunnel as this returns.
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
