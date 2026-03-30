//! PingScheduler — background Tokio task that periodically health-checks running skills.
//!
//! Every 5 minutes the scheduler pings all skills whose status is `Running` by
//! sending an RPC `skill/ping` message.  The response is interpreted as follows:
//!
//!   - `null` / `{ ok: true }` → healthy, no action
//!   - `{ ok: false, errorType: "auth" }` → stop the skill and set an error status
//!   - `{ ok: false, errorType: "network" }` → update `connection_status` in the
//!     skill's published state to `"error"` but keep the skill running
//!
//! Architecture follows the same pattern as `CronScheduler`: a background Tokio
//! task with `tokio::select!` for a tick interval + a stop signal via a watch channel.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use serde::Deserialize;
use tokio::sync::watch;

use crate::openhuman::skills::skill_registry::SkillRegistry;
use crate::openhuman::skills::types::{SkillMessage, SkillStatus};

/// Interval between ping sweeps.
const PING_INTERVAL: Duration = Duration::from_secs(60);

/// Per-skill timeout when waiting for a ping reply.
const PING_TIMEOUT: Duration = Duration::from_secs(30);

/// Deserialized result from a skill's `onPing()` handler.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PingResult {
    ok: bool,
    #[serde(default)]
    error_type: Option<String>,
    #[serde(default)]
    error_message: Option<String>,
}

/// Background ping scheduler that health-checks running skills.
pub struct PingScheduler {
    /// Reference to the skill registry (set after engine initialisation).
    registry: Arc<RwLock<Option<Arc<SkillRegistry>>>>,
    /// Watch channel to signal the tick loop to stop.
    stop_tx: watch::Sender<bool>,
}

impl PingScheduler {
    pub fn new() -> Self {
        let (stop_tx, _) = watch::channel(false);
        Self {
            registry: Arc::new(RwLock::new(None)),
            stop_tx,
        }
    }

    /// Set the skill registry (called after engine initialisation).
    pub fn set_registry(&self, registry: Arc<SkillRegistry>) {
        *self.registry.write() = Some(registry);
    }

    /// Start the background ping loop. Returns the Tokio task handle.
    ///
    /// Must be called from within a Tokio runtime context.
    pub fn start(&self) -> tokio::task::JoinHandle<()> {
        let registry = self.registry.clone();
        let mut stop_rx = self.stop_tx.subscribe();

        tokio::spawn(async move {
            log::info!(
                "[ping] Scheduler started ({}s interval)",
                PING_INTERVAL.as_secs()
            );

            loop {
                tokio::select! {
                    _ = tokio::time::sleep(PING_INTERVAL) => {
                        let reg = registry.read().clone();
                        Self::tick(&reg).await;
                    }
                    _ = stop_rx.changed() => {
                        log::info!("[ping] Scheduler stopped");
                        break;
                    }
                }
            }
        })
    }

    /// Stop the scheduler's tick loop.
    #[allow(dead_code)]
    pub fn stop(&self) {
        let _ = self.stop_tx.send(true);
    }

    /// Ping all running skills concurrently and act on failures.
    async fn tick(registry: &Option<Arc<SkillRegistry>>) {
        let registry = match registry {
            Some(r) => r,
            None => return,
        };

        // Collect skill IDs that are running AND actively connected.
        // Skills in setup mode or not yet connected are excluded to avoid
        // interfering with the setup flow.
        let running: Vec<String> = registry
            .list_skills()
            .into_iter()
            .filter(|s| {
                s.status == SkillStatus::Running
                    && s.state
                        .get("connection_status")
                        .and_then(|v| v.as_str())
                        .is_some_and(|cs| cs == "connected")
            })
            .map(|s| s.skill_id)
            .collect();

        if running.is_empty() {
            return;
        }

        log::debug!("[ping] Pinging {} running skill(s)", running.len());

        // Ping all skills concurrently
        let futures: Vec<_> = running
            .into_iter()
            .map(|skill_id| {
                let registry = Arc::clone(registry);
                async move {
                    Self::ping_skill(&skill_id, &registry).await;
                }
            })
            .collect();

        futures::future::join_all(futures).await;
    }

    /// Ping a single skill and handle the result.
    async fn ping_skill(
        skill_id: &str,
        registry: &Arc<SkillRegistry>,
    ) {
        log::debug!("[ping] Pinging skill '{}'", skill_id);

        // Send the RPC message
        let (tx, rx) = tokio::sync::oneshot::channel();
        if let Err(e) = registry.send_message(
            skill_id,
            SkillMessage::Rpc {
                method: "skill/ping".to_string(),
                params: serde_json::json!({}),
                reply: tx,
            },
        ) {
            log::warn!("[ping] Failed to send ping to '{}': {}", skill_id, e);
            return;
        }

        // Wait for the reply with a timeout
        let reply = match tokio::time::timeout(PING_TIMEOUT, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                log::warn!("[ping] Ping channel closed for '{}'", skill_id);
                return;
            }
            Err(_) => {
                log::warn!("[ping] Ping timed out for '{}'", skill_id);
                return;
            }
        };

        // Parse the result
        let value = match reply {
            Ok(v) => v,
            Err(e) => {
                log::warn!("[ping] Ping RPC error for '{}': {}", skill_id, e);
                return;
            }
        };

        // null / { ok: true } → healthy
        if value.is_null() {
            return;
        }

        let ping_result: PingResult = match serde_json::from_value(value) {
            Ok(r) => r,
            Err(e) => {
                log::debug!(
                    "[ping] Could not parse ping result for '{}': {} — treating as healthy",
                    skill_id,
                    e
                );
                return;
            }
        };

        if ping_result.ok {
            return;
        }

        // ----- Handle failure -----
        let error_type = ping_result.error_type.as_deref().unwrap_or("unknown");
        let error_message = ping_result
            .error_message
            .as_deref()
            .unwrap_or("Ping failed");

        log::warn!(
            "[ping] Skill '{}' ping failed: type={}, message={}",
            skill_id,
            error_type,
            error_message
        );

        match error_type {
            "auth" => {
                // Auth failure: stop the skill and emit error event
                log::info!("[ping] Stopping skill '{}' due to auth failure", skill_id);

                if let Err(e) = registry.stop_skill(skill_id).await {
                    log::error!("[ping] Failed to stop skill '{}': {}", skill_id, e);
                }
            }
            _ => {
                // Network or other error: update published state, keep running
                if let Some(snap) = registry.get_skill(skill_id) {
                    // We need to update the skill's published_state through the
                    // registry. The SkillState is behind an Arc<RwLock<>>, which
                    // we can reach via the snapshot's backing state. However, the
                    // registry only exposes snapshots (copies). We use an RPC
                    // message to let the skill instance update its own state.
                    //
                    // A simpler approach: directly update published_state via the
                    // SkillState Arc that the registry entry holds. Since
                    // SkillRegistry doesn't expose the Arc directly, we send a
                    // state/set RPC to the skill, which is the same mechanism
                    // the frontend uses.
                    let _ = snap; // used for logging context

                    // Send a state update via RPC (skills handle "state/set"
                    // in their reverse-RPC handler, but here we update the
                    // published_state directly through the skill message loop).
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    let _ = registry.send_message(
                        skill_id,
                        SkillMessage::Rpc {
                            method: "state/set".to_string(),
                            params: serde_json::json!({
                                "partial": {
                                    "connection_status": "error",
                                    "connection_error": error_message,
                                }
                            }),
                            reply: tx,
                        },
                    );
                    // Don't block on the reply — fire-and-forget
                    let _ = tokio::time::timeout(Duration::from_secs(5), rx).await;
                }
            }
        }
    }
}

impl Default for PingScheduler {
    fn default() -> Self {
        Self::new()
    }
}
