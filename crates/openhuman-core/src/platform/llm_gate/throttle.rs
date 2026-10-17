//! The cached policy, the background sampler, and cooperative throttling.
//!
//! One sampler task refreshes [`Signals`] every [`SAMPLE_INTERVAL`] and
//! recomputes the [`Policy`]. Workers call [`wait_for_capacity`] to
//! cooperatively block until the host is ready.
//!
//! Nothing here is process-global: the host owns the [`SharedCore`], the
//! semaphore and the signed-out flag, and hands them in. That is what lets it
//! give every unit test its own.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::decide::{decide, Signals};
use super::signals::{self, SignalEnv};
use crate::config::schema::{Policy, SchedulerGateConfig};

/// Process-wide ceiling on concurrent LLM-bound work.
///
/// Held at 1 to keep concurrent local-Ollama / bge-m3 calls (8K context,
/// ~1.3 GB resident each) from saturating local RAM — backfills with multiple
/// simultaneous Ollama requests have crashed a laptop twice.
///
/// Cloud-backend LLM calls bypass this semaphore at the worker layer because
/// they're bandwidth-bound, not RAM-bound, and the worker pool itself bounds
/// concurrency upstream. Keeping this at 1 preserves the laptop-RAM contract
/// regardless of backend.
pub const LLM_SLOTS: usize = 1;

/// How often the background sampler refreshes the signals.
pub const SAMPLE_INTERVAL: Duration = Duration::from_secs(30);

/// A fresh [`LLM_SLOTS`]-slot semaphore for [`wait_for_capacity`].
pub fn new_llm_slots() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(LLM_SLOTS))
}

/// RAII guard returned by [`wait_for_capacity`] / [`acquire_llm_permit`].
///
/// While the caller holds an `LlmPermit`, no other LLM-bound caller sharing the
/// semaphore can acquire one. Drop the permit as soon as the LLM request
/// returns — holding it past post-processing serialises unrelated work for no
/// reason.
///
/// This type is intentionally opaque: callers can't reach into the underlying
/// [`OwnedSemaphorePermit`] and risk forgetting to release it.
#[must_use = "drop the LlmPermit only after the LLM call returns"]
pub struct LlmPermit {
    _permit: OwnedSemaphorePermit,
}

impl Drop for LlmPermit {
    fn drop(&mut self) {
        log::trace!("[scheduler_gate] llm permit released");
    }
}

/// The sampled signals, the user's config and the policy decided from them.
#[derive(Debug)]
pub struct GateCore {
    cfg: SchedulerGateConfig,
    signals: Signals,
    policy: Policy,
}

/// A [`GateCore`] shared between the sampler task and its readers.
pub type SharedCore = Arc<RwLock<GateCore>>;

impl GateCore {
    /// Decide the initial policy from `signals` under `cfg` and log it.
    pub fn new(cfg: SchedulerGateConfig, signals: Signals) -> Self {
        let policy = decide(&signals, &cfg);
        log::info!(
            "[scheduler_gate] startup policy={} mode={} on_ac={} charge={:?} cpu={:.1}% server={}",
            policy.as_str(),
            cfg.mode.as_str(),
            signals.on_ac_power,
            signals.battery_charge,
            signals.cpu_usage_pct,
            signals.server_mode,
        );
        Self {
            cfg,
            signals,
            policy,
        }
    }

    /// The cached policy.
    pub fn policy(&self) -> Policy {
        self.policy
    }

    /// The most recent sampled signals.
    pub fn signals(&self) -> Signals {
        self.signals
    }

    /// The config the policy is decided under.
    pub fn config(&self) -> &SchedulerGateConfig {
        &self.cfg
    }

    /// Replace the config and recompute the policy.
    ///
    /// Returns `true` when this update moved the policy **out of** a paused
    /// state, so the caller can wake parked background loops.
    pub fn update_config(&mut self, cfg: SchedulerGateConfig) -> bool {
        let was_paused = matches!(self.policy, Policy::Paused { .. });
        self.cfg = cfg;
        self.policy = decide(&self.signals, &self.cfg);
        was_paused && !matches!(self.policy, Policy::Paused { .. })
    }

    /// Take a fresh sample, recompute the policy, and log a transition.
    pub fn refresh(&mut self, signals: Signals) {
        let next = decide(&signals, &self.cfg);
        if next != self.policy {
            log::info!(
                "[scheduler_gate] policy {} -> {} (on_ac={} charge={:?} cpu={:.1}% server={})",
                self.policy.as_str(),
                next.as_str(),
                signals.on_ac_power,
                signals.battery_charge,
                signals.cpu_usage_pct,
                signals.server_mode,
            );
        }
        self.signals = signals;
        self.policy = next;
    }
}

/// Spawn the background sampler on the current tokio runtime: every
/// [`SAMPLE_INTERVAL`] it samples the host and [`GateCore::refresh`]es `core`.
pub fn spawn_sampler(core: SharedCore, env: SignalEnv) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(SAMPLE_INTERVAL).await;
            // Sampling does a brief blocking sleep + sysinfo refresh — push it
            // off the async runtime.
            let signals = match tokio::task::spawn_blocking(move || signals::sample(&env)).await {
                Ok(s) => s,
                Err(err) => {
                    log::warn!("[scheduler_gate] sampler join error: {err:#}");
                    continue;
                }
            };
            core.write().refresh(signals);
        }
    });
}

/// Cooperatively block a caller until the host is ready for LLM-bound work,
/// then hand back an [`LlmPermit`] that holds a slot in `slots`.
///
/// `core` is `None` when the host has not initialised a gate (unit tests, early
/// bootstrap): there is no policy to consult, so the permit is acquired
/// directly. `signed_out` is the host's override; it is only honoured once a
/// core exists, because the override only has meaning when there are real
/// background workers calling into the gate.
///
/// Policy-driven backoff happens **before** semaphore acquisition so a `Paused`
/// mode doesn't pile up tasks queued for the slot — they sit in the pause-poll
/// loop, not in the semaphore wait queue.
///
/// * **Aggressive / Normal** — wait for the slot; return once granted.
/// * **Throttled** — sleep `throttled_backoff_ms` first so concurrent workers
///   serialise themselves, then acquire the slot.
/// * **Paused** — poll every `paused_poll_ms` until the policy changes, then
///   acquire the slot.
///
/// Returns `None` only if the semaphore has been closed. Callers can safely
/// treat `None` as "skip the gate" rather than propagating an error.
pub async fn wait_for_capacity(
    core: Option<&SharedCore>,
    signed_out: impl Fn() -> bool,
    slots: &Arc<Semaphore>,
) -> Option<LlmPermit> {
    loop {
        // Signed-out override is checked first and uses the same paused-poll
        // cadence as the rest of the Paused arm. Holding here (rather than
        // returning) means workers naturally resume the instant the user signs
        // back in — no respawn dance, no missed wakeups.
        if let Some(core) = core {
            if signed_out() {
                let paused_ms = core.read().cfg.paused_poll_ms;
                log::trace!("[scheduler_gate] paused (signed_out); polling every {paused_ms}ms");
                tokio::time::sleep(Duration::from_millis(paused_ms)).await;
                continue;
            }
        }

        let (policy, throttled_ms, paused_ms) = match core {
            Some(core) => {
                let g = core.read();
                (g.policy, g.cfg.throttled_backoff_ms, g.cfg.paused_poll_ms)
            }
            // Gate not initialised: acquire directly — no policy to consult.
            None => return acquire_llm_permit(slots).await,
        };
        match policy {
            Policy::Aggressive | Policy::Normal => return acquire_llm_permit(slots).await,
            Policy::Throttled => {
                log::trace!(
                    "[scheduler_gate] throttled — sleeping {throttled_ms}ms before permit acquire"
                );
                tokio::time::sleep(Duration::from_millis(throttled_ms)).await;
                return acquire_llm_permit(slots).await;
            }
            Policy::Paused { reason } => {
                log::debug!(
                    "[scheduler_gate] paused ({}); polling every {paused_ms}ms",
                    reason.as_str()
                );
                tokio::time::sleep(Duration::from_millis(paused_ms)).await;
                // re-evaluate; user may have toggled the gate back on.
            }
        }
    }
}

/// Acquire a slot in `slots` without consulting any policy. Production callers
/// should use [`wait_for_capacity`] so the policy backoff applies.
pub async fn acquire_llm_permit(slots: &Arc<Semaphore>) -> Option<LlmPermit> {
    match slots.clone().acquire_owned().await {
        Ok(permit) => {
            log::trace!("[scheduler_gate] llm permit acquired");
            Some(LlmPermit { _permit: permit })
        }
        Err(_) => {
            // Semaphore closed — should never happen since nothing closes it.
            // Log loudly and let the caller proceed without a permit so the
            // pipeline doesn't deadlock.
            log::warn!(
                "[scheduler_gate] llm semaphore closed unexpectedly — proceeding without a permit"
            );
            None
        }
    }
}

/// Try to grab a slot without waiting or consulting the policy. `None` if no
/// slot is free.
pub fn try_acquire_llm_permit(slots: &Arc<Semaphore>) -> Option<LlmPermit> {
    slots
        .clone()
        .try_acquire_owned()
        .ok()
        .map(|p| LlmPermit { _permit: p })
}

#[cfg(test)]
#[path = "throttle_tests.rs"]
mod tests;
