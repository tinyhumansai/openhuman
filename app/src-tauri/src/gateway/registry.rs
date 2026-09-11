//! Which gateway is active, and what is being held open for it.
//!
//! A provisioned gateway is only as alive as the values holding it open: drop
//! the [`Forward`](tinybox_core::Forward) and the tunnel closes. So the process
//! needs somewhere to keep them for as long as the gateway is selected, and
//! that is here.
//!
//! Everything is behind one lock and every write goes through
//! [`activate`], so two screens racing to switch gateways cannot leave the
//! process holding a tunnel to a box nothing points at.

use std::sync::Arc;

use tokio::sync::Mutex;

use super::ops::{self, Provisioned};
use super::types::{ActiveGateway, Gateway, GatewayStatus, DESKTOP_ID};

/// The active gateway and whatever is keeping it alive.
#[derive(Default)]
struct State {
    /// Where RPC currently goes. `None` before the first activation, which the
    /// desktop fallback in [`current`] covers.
    active: Option<ActiveGateway>,
    /// Held open for as long as this gateway is selected.
    provisioned: Option<Provisioned>,
    /// What to show while activation is in flight, and after it fails.
    status: Option<GatewayStatus>,
    /// Monotonic counter bumping on every activation start. Progress updates
    /// capture the value they belong to so a delayed update from a superseded
    /// activation cannot resurrect `Activating` over a terminal state.
    generation: u64,
}

static STATE: std::sync::LazyLock<Arc<Mutex<State>>> =
    std::sync::LazyLock::new(|| Arc::new(Mutex::new(State::default())));

/// Serializes the whole `activate` transaction.
///
/// `STATE` protects individual fields but not a full activation: if two
/// activations interleave, a slower one that finishes last would clobber the
/// active gateway the user actually selected, and then tear down the newer
/// box while RPC still points at it. Holding this lock across the entire
/// sequence — provisioning, status commit, and previous-gateway teardown —
/// means the last activation to start is also the last to finish.
static ACTIVATION_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));

/// Where RPC should currently go.
///
/// Falls back to the in-process core when nothing has been activated — which is
/// the state at launch, and the state after a failed activation. There is
/// always a working answer, because the core in this process is always
/// reachable.
pub async fn current(desktop: &crate::core_process::CoreProcessHandle) -> ActiveGateway {
    if let Some(active) = STATE.lock().await.active.clone() {
        return active;
    }
    ActiveGateway {
        id: DESKTOP_ID.to_owned(),
        rpc_url: desktop.rpc_url(),
        token: Some(desktop.rpc_token().to_owned()),
    }
}

/// The active gateway's id, without resolving its endpoint.
pub async fn active_id() -> String {
    STATE
        .lock()
        .await
        .active
        .as_ref()
        .map_or_else(|| DESKTOP_ID.to_owned(), |active| active.id.clone())
}

/// What `id` is doing right now.
pub async fn status_of(id: &str) -> GatewayStatus {
    let state = STATE.lock().await;
    let is_active = state
        .active
        .as_ref()
        .map_or(id == DESKTOP_ID, |active| active.id == id);

    match (&state.status, is_active) {
        // A recorded status only describes the gateway the last activation was
        // for; reporting it for a different one would show a failure against
        // whichever gateway the user happened to be looking at.
        (Some(status), true) => status.clone(),
        (_, true) => GatewayStatus::Connected {
            endpoint: state.active.as_ref().map_or_else(
                || "the local core".to_owned(),
                |active| crate::core_rpc::redact_url_for_log(&active.rpc_url),
            ),
        },
        (_, false) => GatewayStatus::Inactive,
    }
}

/// Make `gateway` the one RPC goes to.
///
/// The previous gateway is torn down **after** the new one is up, so a failed
/// activation leaves the working one in place rather than stranding the user
/// with nothing. That ordering costs one overlapping box and is worth it: the
/// alternative is that a typo in an SSH destination takes the app offline.
///
/// # Errors
///
/// Returns a user-facing message when the gateway cannot be reached. The
/// previously active gateway is still active in that case.
pub async fn activate(
    gateway: &Gateway,
    desktop: &crate::core_process::CoreProcessHandle,
) -> Result<ActiveGateway, String> {
    let _activation_guard = ACTIVATION_LOCK.lock().await;
    let generation = {
        let mut state = STATE.lock().await;
        state.generation += 1;
        let generation = state.generation;
        state.status = Some(GatewayStatus::Activating {
            step: "starting".to_owned(),
        });
        generation
    };

    let progress = {
        let id = gateway.id.clone();
        move |step: &str| {
            log::debug!("[gateway][activate] {id}: {step}");
            let step = step.to_owned();
            // A detached task because progress is reported from inside
            // provisioning, which holds no lock and must not start waiting on
            // one to say what it is doing.
            tokio::spawn(async move {
                let mut state = STATE.lock().await;
                // A progress update must never overwrite a terminal state
                // (Connected/Failed) written by this or a later activation, or
                // resurrect `Activating` for one that has been superseded.
                if state.generation == generation
                    && !matches!(
                        state.status,
                        Some(GatewayStatus::Connected { .. } | GatewayStatus::Failed { .. })
                    )
                {
                    state.status = Some(GatewayStatus::Activating { step });
                }
            });
        }
    };

    let outcome = ops::activate(gateway, desktop, &progress).await;

    let provisioned = match outcome {
        Ok(provisioned) => provisioned,
        Err(reason) => {
            log::warn!("[gateway][activate] {} failed: {reason}", gateway.id);
            STATE.lock().await.status = Some(GatewayStatus::Failed {
                reason: reason.clone(),
            });
            return Err(reason);
        }
    };

    // `ops::activate` provisions exactly the specs `endpoint_of` declines, so
    // one of the two always answers. Stating that through `provisions()`
    // rather than inferring it from a `None` means a future spec variant that
    // forgets one half fails here, visibly, instead of silently resolving to
    // whichever branch happened to be reachable.
    let active = match &provisioned {
        Some(provisioned) => provisioned.active.clone(),
        None => match ops::endpoint_of(gateway, desktop) {
            Some(endpoint) => endpoint,
            None => {
                let reason = if gateway.spec.provisions() {
                    "this gateway should have been provisioned and was not".to_owned()
                } else {
                    "this gateway resolved to no endpoint".to_owned()
                };
                STATE.lock().await.status = Some(GatewayStatus::Failed {
                    reason: reason.clone(),
                });
                return Err(reason);
            }
        },
    };

    let previous = {
        let mut state = STATE.lock().await;
        let previous = state.provisioned.take();
        state.active = Some(active.clone());
        state.provisioned = provisioned;
        state.status = Some(GatewayStatus::Connected {
            endpoint: crate::core_rpc::redact_url_for_log(&active.rpc_url),
        });
        previous
    };

    if let Some(previous) = previous {
        log::debug!("[gateway][activate] tearing down the previous gateway");
        previous.tear_down().await;
    }

    log::info!("[gateway][activate] {} is now active", gateway.id);
    Ok(active)
}

/// Tear down whatever is held open, on the way out of the process.
pub async fn shutdown() {
    let provisioned = STATE.lock().await.provisioned.take();
    if let Some(provisioned) = provisioned {
        log::info!("[gateway][shutdown] tearing down the active gateway");
        provisioned.tear_down().await;
    }
}
