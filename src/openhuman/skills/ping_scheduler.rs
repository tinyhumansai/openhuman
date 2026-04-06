//! `PingScheduler` — background Tokio task that periodically health-checks running skills.
//!
//! Every minute, the scheduler pings all skills with a status of `Running` by
//! sending an RPC `skill/ping` message. The response is interpreted as follows:
//! - `null` / `{ ok: true }` → healthy, no action required.
//! - `{ ok: false, errorType: "auth" }` → stop the skill and set an error status (authentication failure).
//! - `{ ok: false, errorType: "network" }` → update `connection_status` in the skill's published state to `"error"`, but keep the skill running.
//!
//! This ensures that running skills are responsive and can report their internal health state.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use serde::Deserialize;
use tokio::sync::watch;

use crate::openhuman::skills::skill_registry::SkillRegistry;
use crate::openhuman::skills::types::{SkillMessage, SkillStatus};

/// The interval between consecutive health-check sweeps across all skills.
const PING_INTERVAL: Duration = Duration::from_secs(60);

/// The maximum time allowed for a skill to respond to a ping RPC.
const PING_TIMEOUT: Duration = Duration::from_secs(30);

/// Structure for parsing the result returned by a skill's native `onPing()` handler.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PingResult {
    /// Whether the skill reports itself as healthy.
    ok: bool,
    /// Categorization of the error if `ok` is false (e.g., "auth", "network").
    #[serde(default)]
    error_type: Option<String>,
    /// A descriptive error message provided by the skill.
    #[serde(default)]
    error_message: Option<String>,
}

/// A scheduler that manages the health-monitoring lifecycle of running skills.
pub struct PingScheduler {
    /// A reference to the global skill registry, used to list skills and send ping messages.
    registry: Arc<RwLock<Option<Arc<SkillRegistry>>>>,
    /// A channel used to signal the background tick loop to stop.
    stop_tx: watch::Sender<bool>,
}

impl PingScheduler {
    /// Creates a new `PingScheduler` instance.
    pub fn new() -> Self {
        let (stop_tx, _) = watch::channel(false);
        Self {
            registry: Arc::new(RwLock::new(None)),
            stop_tx,
        }
    }

    /// Sets the skill registry reference for the scheduler.
    pub fn set_registry(&self, registry: Arc<SkillRegistry>) {
        *self.registry.write() = Some(registry);
    }

    /// Starts the background ping monitoring loop as a Tokio task.
    ///
    /// The loop performs a sweep of all running skills every `PING_INTERVAL`.
    /// Returns the handle to the spawned task.
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

    /// Signals the background ping monitoring loop to stop.
    #[allow(dead_code)]
    pub fn stop(&self) {
        let _ = self.stop_tx.send(true);
    }

    /// Performs a single sweep across all running and connected skills.
    ///
    /// Skills that are still in setup or not explicitly "connected" are skipped to avoid
    /// interfering with their initialization.
    async fn tick(registry: &Option<Arc<SkillRegistry>>) {
        let registry = match registry {
            Some(r) => r,
            None => return,
        };

        // Collect IDs of skills that are eligible for health checks.
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

        // Dispatch pings to all eligible skills concurrently.
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

    /// Pings a single skill via RPC and processes its response.
    async fn ping_skill(skill_id: &str, registry: &Arc<SkillRegistry>) {
        log::debug!("[ping] Pinging skill '{}'", skill_id);

        // Send the `skill/ping` RPC message.
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

        // Wait for the skill to reply, with a safety timeout.
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

        // Parse the RPC result.
        let value = match reply {
            Ok(v) => v,
            Err(e) => {
                log::warn!("[ping] Ping RPC error for '{}': {}", skill_id, e);
                return;
            }
        };

        // Interpret the result. A null response is considered a successful health check.
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

        // ----- Handle ping failure reporting -----
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
                // Critical authentication failure: stop the skill immediately.
                log::info!("[ping] Stopping skill '{}' due to auth failure", skill_id);

                if let Err(e) = registry.stop_skill(skill_id).await {
                    log::error!("[ping] Failed to stop skill '{}': {}", skill_id, e);
                }
            }
            _ => {
                // Non-critical network or transient error: update the skill's state.
                let mut patch = HashMap::new();
                patch.insert("connection_status".to_string(), serde_json::json!("error"));
                patch.insert(
                    "connection_error".to_string(),
                    serde_json::json!(error_message),
                );
                if let Err(e) = registry.merge_published_state(skill_id, patch).await {
                    log::warn!(
                        "[ping] Could not merge ping failure into published state for '{}': {}",
                        skill_id,
                        e
                    );
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
