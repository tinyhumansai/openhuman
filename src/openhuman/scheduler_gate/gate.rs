//! Process-wide singleton: cached policy + cooperative throttling.
//!
//! One sampler task refreshes [`Signals`] every 30s and recomputes the
//! [`Policy`]. Workers call [`current_policy`] for cheap reads or
//! [`wait_for_capacity`] to cooperatively block until the host is ready.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use parking_lot::RwLock;

use crate::openhuman::config::{Config, SchedulerGateConfig};
use crate::openhuman::scheduler_gate::policy::{decide, Policy};
use crate::openhuman::scheduler_gate::signals::Signals;

struct State {
    cfg: SchedulerGateConfig,
    signals: Signals,
    policy: Policy,
}

static STATE: OnceLock<Arc<RwLock<State>>> = OnceLock::new();
static STARTED: std::sync::Once = std::sync::Once::new();

const SAMPLE_INTERVAL: Duration = Duration::from_secs(30);

/// Initialise the gate and spawn the background sampler.
///
/// Idempotent — repeat calls during bootstrap are no-ops. Subsequent config
/// reloads should call [`update_config`] instead.
pub fn init_global(config: &Config) {
    let cfg = config.scheduler_gate.clone();
    STARTED.call_once(|| {
        let signals = Signals::sample();
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
        let state = Arc::new(RwLock::new(State {
            cfg,
            signals,
            policy,
        }));
        let _ = STATE.set(state.clone());

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(SAMPLE_INTERVAL).await;
                // Sampling does a brief blocking sleep + sysinfo refresh —
                // push it off the async runtime.
                let signals = match tokio::task::spawn_blocking(Signals::sample).await {
                    Ok(s) => s,
                    Err(err) => {
                        log::warn!("[scheduler_gate] sampler join error: {err:#}");
                        continue;
                    }
                };
                let mut guard = state.write();
                let next = decide(&signals, &guard.cfg);
                if next != guard.policy {
                    log::info!(
                        "[scheduler_gate] policy {} -> {} (on_ac={} charge={:?} cpu={:.1}% server={})",
                        guard.policy.as_str(),
                        next.as_str(),
                        signals.on_ac_power,
                        signals.battery_charge,
                        signals.cpu_usage_pct,
                        signals.server_mode,
                    );
                }
                guard.signals = signals;
                guard.policy = next;
            }
        });
    });
}

/// Update the gate's view of user config (e.g. after a settings change).
pub fn update_config(cfg: SchedulerGateConfig) {
    if let Some(state) = STATE.get() {
        let mut guard = state.write();
        guard.cfg = cfg;
        guard.policy = decide(&guard.signals, &guard.cfg);
    }
}

/// Current policy. Defaults to [`Policy::Normal`] before [`init_global`] runs
/// (e.g. in unit tests) so callers don't deadlock waiting on a sampler that
/// will never start.
pub fn current_policy() -> Policy {
    STATE
        .get()
        .map(|s| s.read().policy)
        .unwrap_or(Policy::Normal)
}

/// Most recent sampled signals, or a neutral default if the sampler hasn't run.
pub fn current_signals() -> Signals {
    STATE.get().map(|s| s.read().signals).unwrap_or(Signals {
        on_ac_power: true,
        battery_charge: None,
        cpu_usage_pct: 0.0,
        server_mode: false,
    })
}

/// Cooperatively block a worker until the host is ready for LLM-bound work.
///
/// * **Aggressive / Normal** — returns immediately.
/// * **Throttled** — sleeps `throttled_backoff_ms` so concurrent workers
///   serialise themselves and the host catches its breath between jobs.
/// * **Paused** — polls every `paused_poll_ms` until the policy changes.
///
/// Designed so existing semaphore-bounded worker pools can keep their pool
/// size and just gain a per-job throttle in front of the existing
/// `semaphore.acquire()` call.
pub async fn wait_for_capacity() {
    loop {
        let (policy, throttled_ms, paused_ms) = match STATE.get() {
            Some(state) => {
                let g = state.read();
                (g.policy, g.cfg.throttled_backoff_ms, g.cfg.paused_poll_ms)
            }
            None => return,
        };
        match policy {
            Policy::Aggressive | Policy::Normal => return,
            Policy::Throttled => {
                tokio::time::sleep(Duration::from_millis(throttled_ms)).await;
                return;
            }
            Policy::Paused => {
                tokio::time::sleep(Duration::from_millis(paused_ms)).await;
                // re-evaluate; user may have toggled the gate back on.
            }
        }
    }
}
