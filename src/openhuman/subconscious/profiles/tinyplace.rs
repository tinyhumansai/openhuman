//! The `tinyplace` subconscious world — tiny.place orchestration steering.
//!
//! Reflects offline over the orchestration layer's 20:1-compressed execution
//! history + cumulative world-state diff and, when a macro-trend warrants it,
//! emits **one** `STEERING_DIRECTIVE` that later reasoning cycles inject into
//! their prompt. The reflect turn is a **tool-free provider chat** — it
//! constructs no Agent and no toolset, so it can never contact anyone (the
//! isolation invariant, enforced by a source scan below).
//!
//! This profile is pure scheduler + policy: the orchestration domain still owns
//! its store shapes and the steering contract via
//! [`orchestration::ops::load_review_window`] / [`synthesize_and_persist`].

use std::sync::Arc;

use async_trait::async_trait;
use tracing::warn;

use super::super::instance::SubconsciousInstance;
use super::super::profile::{Observation, Reflection, SubconsciousProfile};
use crate::openhuman::agent::turn_origin::TrustedAutomationSource;
use crate::openhuman::config::Config;
use crate::openhuman::orchestration::ops::{load_review_window, synthesize_and_persist};
use crate::openhuman::orchestration::store as orch_store;

/// Construct the live `tinyplace` instance from config (used by the registry /
/// bootstrap). The only place `TinyPlaceProfile` is wired into a runner.
pub fn tinyplace_instance(config: &Config) -> SubconsciousInstance {
    let interval = config
        .orchestration
        .effective_review_interval_minutes(config.heartbeat.interval_minutes);
    SubconsciousInstance::new(
        Arc::new(TinyPlaceProfile),
        config.workspace_dir.clone(),
        config.orchestration.enabled,
        interval,
        // Steering has no mode concept; a fixed label keeps the status shape.
        "steering",
    )
}

/// The `tinyplace` world profile. Stateless — every tick reads its window live
/// from the orchestration store through the shared ops seam.
pub struct TinyPlaceProfile;

#[async_trait]
impl SubconsciousProfile for TinyPlaceProfile {
    fn id(&self) -> &'static str {
        "tinyplace"
    }

    fn cadence(&self, config: &Config) -> std::time::Duration {
        let mins = config
            .orchestration
            .effective_review_interval_minutes(config.heartbeat.interval_minutes);
        std::time::Duration::from_secs(u64::from(mins) * 60)
    }

    async fn observe(&self, config: &Config) -> Observation {
        // Self-gating: `None` when orchestration is disabled or nothing new to
        // review (same idle gate as the pre-factory `Ok(false)` path).
        match load_review_window(config).await {
            Ok(Some(window)) => {
                let commit_token = window.newest_reviewed.clone();
                // Serialize the whole window through the tick graph's state so
                // reflect sees exactly the rows observe pinned (the checkpoint
                // schema stays a single string, per the generic Observation).
                let rendered = match serde_json::to_string(&window) {
                    Ok(json) => json,
                    Err(e) => {
                        warn!("[subconscious:tinyplace] review window encode failed: {e}");
                        return Observation::default();
                    }
                };
                Observation {
                    rendered,
                    has_changes: true,
                    // Harness DMs are third-party content — always tainted.
                    has_external_content: true,
                    commit_token,
                }
            }
            Ok(None) => Observation::default(),
            Err(e) => {
                warn!("[subconscious:tinyplace] review load failed: {e}");
                Observation::default()
            }
        }
    }

    // prepare_context: default no-op — steering is deliberately tool-free.

    async fn reflect(
        &self,
        config: &Config,
        obs: &Observation,
        _prepared_context: &str,
    ) -> Result<Reflection, String> {
        let window = serde_json::from_str(&obs.rendered)
            .map_err(|e| format!("review window decode: {e}"))?;
        let tick_id = format!("subconscious:tinyplace:{}", now_secs() as u64);
        match synthesize_and_persist(config, &window, &tick_id).await? {
            Some(directive_id) => Ok(Reflection::Steered { directive_id }),
            // A clean NONE or twice-failed synthesis is an idle result, not an
            // error — the cursor still advances (via commit) so we don't reflect
            // the same rows forever.
            None => Ok(Reflection::Idle),
        }
    }

    async fn commit(&self, config: &Config, obs: &Observation) {
        // Advance the review cursor to exactly the window observed. Only the
        // runner's non-superseded, non-failed path reaches here, so this is the
        // uniform "advance only when the tick stuck" point. A quiet tick carries
        // no token → no-op.
        if let Some(token) = &obs.commit_token {
            if let Err(e) = orch_store::with_connection(&config.workspace_dir, |conn| {
                orch_store::set_review_cursor(conn, token)
            }) {
                warn!("[subconscious:tinyplace] review cursor advance failed: {e}");
            }
        }
    }

    fn origin(&self, _obs: &Observation) -> TrustedAutomationSource {
        // Steering always reacts to third-party harness content.
        TrustedAutomationSource::SubconsciousTainted
    }
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
#[path = "tinyplace_tests.rs"]
mod tests;
