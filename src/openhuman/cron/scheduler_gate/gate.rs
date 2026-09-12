//! Process-wide singleton: cached policy + cooperative throttling.
//!
//! One sampler task refreshes [`Signals`] every 30s and recomputes the
//! [`Policy`]. Workers call [`current_policy`] for cheap reads or
//! [`wait_for_capacity`] to cooperatively block until the host is ready.

#[cfg(not(test))]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use parking_lot::RwLock;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};

use crate::openhuman::config::{Config, SchedulerGateConfig};
use crate::openhuman::cron::scheduler_gate::policy::{decide, PauseReason, Policy};
use crate::openhuman::cron::scheduler_gate::signals::Signals;

/// Process-wide ceiling on concurrent LLM-bound work.
///
/// Held at 1 to keep concurrent local-Ollama / bge-m3 calls (8K context,
/// ~1.3 GB resident each) from saturating local RAM. See
/// `feedback_local_llm_load.md` — backfills with multiple
/// simultaneous Ollama requests have crashed the user's laptop twice.
///
/// Cloud-backend LLM calls bypass this semaphore at the worker layer
/// (see `memory_queue::worker::run_once`) because they're
/// bandwidth-bound, not RAM-bound, and the worker pool itself bounds
/// concurrency upstream. Keeping this at 1 preserves the laptop-RAM
/// contract regardless of backend.
const LLM_SLOTS: usize = 1;

#[cfg(not(test))]
static LLM_PERMITS: OnceLock<Arc<Semaphore>> = OnceLock::new();

/// Hand back the semaphore that gates concurrent LLM work.
///
/// **Production**: one process-wide `Arc<Semaphore>` — the laptop-RAM
/// safety contract documented on `LLM_SLOTS`.
///
/// **Tests**: one `Arc<Semaphore>` per tokio runtime, keyed by
/// `tokio::runtime::Handle::current().id()` (see [`test_state`]).
/// Each `#[tokio::test]` builds a fresh runtime → fresh id → fresh
/// slot, immune to both cross-thread contention from parallel cargo
/// workers and to libtest's reuse of the same OS thread for
/// successive tests. The single-slot invariant (and behaviour
/// tied to it) is still observable *within* a test because every
/// task that test spawns runs on the same runtime → same id →
/// same `Arc<Semaphore>`.
#[cfg(not(test))]
fn llm_permits() -> Arc<Semaphore> {
    LLM_PERMITS
        .get_or_init(|| Arc::new(Semaphore::new(LLM_SLOTS)))
        .clone()
}

/// Per-tokio-runtime gate state for the unit-test build.
///
/// Both [`LLM_PERMITS`] and [`SIGNED_OUT`] are conceptually process-
/// wide in production, but cargo runs `#[tokio::test]`s in parallel
/// (cross-thread contention on the semaphore) AND recycles the
/// libtest OS threads across tests (thread-local state leaks
/// state from `credentials::*` tests that toggle `SIGNED_OUT` into
/// later tests on the same thread). Keying by
/// `tokio::runtime::Handle::current().id()` sidesteps both: every
/// `#[tokio::test]` builds a fresh runtime and gets its own slot,
/// regardless of which libtest worker thread happens to host it.
///
/// The map grows monotonically over a test run (one entry per
/// runtime created); that's fine — a full lib-test pass is well
/// under 10k entries and the process exits when it finishes.
#[cfg(test)]
#[path = "gate_test_state_tests.rs"]
mod test_state;

/// Process-wide fallback semaphore for synchronous tests that have no
/// tokio runtime. Async tests get a per-runtime semaphore (see
/// [`test_state`]) so they can't contend across tests.
#[cfg(test)]
static FALLBACK_LLM_PERMITS: OnceLock<Arc<Semaphore>> = OnceLock::new();

#[cfg(test)]
fn llm_permits() -> Arc<Semaphore> {
    match test_state::current_id() {
        Some(id) => test_state::permits_for(id),
        None => FALLBACK_LLM_PERMITS
            .get_or_init(|| Arc::new(Semaphore::new(LLM_SLOTS)))
            .clone(),
    }
}

/// RAII guard returned by [`wait_for_capacity`] / [`acquire_llm_permit`].
///
/// While the caller holds an `LlmPermit`, no other LLM-bound caller in
/// the process can acquire one (the global semaphore has a single slot).
/// Drop the permit as soon as the LLM request returns — holding it past
/// post-processing serialises unrelated work for no reason.
///
/// This type is intentionally opaque: callers can't reach into the
/// underlying [`OwnedSemaphorePermit`] and risk forgetting to release it.
#[must_use = "drop the LlmPermit only after the LLM call returns"]
pub struct LlmPermit {
    _permit: OwnedSemaphorePermit,
}

impl Drop for LlmPermit {
    fn drop(&mut self) {
        log::trace!("[scheduler_gate] llm permit released");
    }
}

struct State {
    cfg: SchedulerGateConfig,
    signals: Signals,
    policy: Policy,
}

static STATE: OnceLock<Arc<RwLock<State>>> = OnceLock::new();
static STARTED: std::sync::Once = std::sync::Once::new();

/// Process-wide "session is signed out" override. When `true`, every gate
/// query returns [`Policy::Paused`] with [`PauseReason::SignedOut`],
/// regardless of host signals or config. This is the kill switch the
/// credentials lifecycle and 401-detection sites use to halt background
/// LLM work the moment the session goes away — without it, cron / channel
/// loops keep firing requests at a backend that will only ever 401 them.
///
/// Default is `false` (assume signed in). `init_global` reseats it from
/// the on-disk session at startup, and `store_session` / `clear_session`
/// toggle it through [`set_signed_out`].
#[cfg(not(test))]
static SIGNED_OUT: AtomicBool = AtomicBool::new(false);

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

/// Process-wide resume signal (#2831). Fired whenever the gate transitions
/// **out of** a paused state — the user toggles Memory Tree back on
/// ([`update_config`]) or signs back in ([`set_signed_out`]). Background loops
/// (e.g. the Composio periodic scheduler) park on [`resume_notify`] so they can
/// resume work within seconds instead of waiting out their next tick boundary.
static RESUME_NOTIFY: OnceLock<Arc<Notify>> = OnceLock::new();

/// Handle to the process-wide resume [`Notify`] (#2831).
///
/// Both the firing side (`update_config` / `set_signed_out`) and the waiting
/// side (background loops) call this, so they share one instance. Use
/// `notify_one()` to fire: if a loop is parked it wakes immediately; if it's
/// mid-tick, a single permit is stored so the *next* `notified()` returns at
/// once — a resume that arrives during a tick is never lost.
///
/// **Over-notifying is safe by design.** A spurious wake (e.g. Memory Tree
/// toggled on while still signed out, so the effective policy is still paused)
/// just causes one cheap gate-checked tick that re-reads [`current_policy`] and
/// no-ops. We therefore fire on each individual un-pause transition rather than
/// computing the precise combined (config × signed-out) edge.
pub fn resume_notify() -> Arc<Notify> {
    RESUME_NOTIFY
        .get_or_init(|| Arc::new(Notify::new()))
        .clone()
}

/// Update the gate's view of user config (e.g. after a settings change).
///
/// Fires [`resume_notify`] when this update moves the policy out of a paused
/// state (e.g. Memory Tree toggled back on), so parked background loops resume
/// promptly (#2831).
pub fn update_config(cfg: SchedulerGateConfig) {
    let Some(state) = STATE.get() else {
        return;
    };
    let resumed = {
        let mut guard = state.write();
        let was_paused = matches!(guard.policy, Policy::Paused { .. });
        guard.cfg = cfg;
        guard.policy = decide(&guard.signals, &guard.cfg);
        was_paused && !matches!(guard.policy, Policy::Paused { .. })
    };
    if resumed {
        resume_notify().notify_one();
    }
}

/// Current policy. Defaults to [`Policy::Normal`] before [`init_global`] runs
/// (e.g. in unit tests) so callers don't deadlock waiting on a sampler that
/// will never start.
///
/// When the signed-out override is set **and the gate has been initialised**,
/// returns [`Policy::Paused`] with [`PauseReason::SignedOut`] — this is the
/// top-priority "host should do no LLM work" signal and ignores config /
/// signals. We gate on [`STATE`] being present because the override only has
/// a meaningful effect when there are real background workers calling into
/// the gate; in unit tests where `init_global` was never called, a stale
/// `signed_out` flag from an earlier test can otherwise deadlock every
/// subsequent caller (see `wait_for_capacity` for the deadlock path).
pub fn current_policy() -> Policy {
    if STATE.get().is_some() && is_signed_out() {
        return Policy::Paused {
            reason: PauseReason::SignedOut,
        };
    }
    STATE
        .get()
        .map(|s| s.read().policy)
        .unwrap_or(Policy::Normal)
}

/// `true` when the signed-out override is active. Cheap atomic load —
/// safe to call from hot paths (e.g. per-LLM-call short-circuit in
/// `OpenHumanBackendModel`).
#[cfg(not(test))]
pub fn is_signed_out() -> bool {
    SIGNED_OUT.load(Ordering::Acquire)
}

#[cfg(test)]
pub fn is_signed_out() -> bool {
    match test_state::current_id() {
        Some(id) => test_state::signed_out_for(id),
        None => false,
    }
}

/// Toggle the signed-out override. Set to `true` from `clear_session`
/// and 401-detection sites; set to `false` from `store_session` once a
/// fresh JWT has been written. Idempotent.
///
/// Gated on [`STATE`] being initialised: if the scheduler gate hasn't
/// been started (every unit-test binary, plus the brief pre-`init_global`
/// window during bootstrap), this is a no-op. There are no background
/// workers to stand down in that state, and unconditionally flipping the
/// process-global atomic lets test paths like `clear_session` and
/// `SessionExpiredSubscriber.handle()` leak `true` into subsequent tests
/// that — if anything later promotes [`STATE`] to `Some` — will spin
/// forever in the `paused_poll_ms` branch of [`wait_for_capacity`].
/// Gating at the writer is a belt-and-braces companion to the reader-side
/// guard added in PR #1552.
#[cfg(not(test))]
pub fn set_signed_out(signed_out: bool) {
    if STATE.get().is_none() {
        return;
    }
    let prev = SIGNED_OUT.swap(signed_out, Ordering::AcqRel);
    if prev != signed_out {
        log::info!("[scheduler_gate] signed_out {} -> {}", prev, signed_out);
        // #2831: signing back in (true -> false) is a transition out of
        // `Policy::Paused { SignedOut }`. Wake any periodic loop so background
        // sync restarts immediately rather than at the next tick boundary.
        if prev && !signed_out {
            resume_notify().notify_one();
        }
    }
}

#[cfg(test)]
pub fn set_signed_out(signed_out: bool) {
    if STATE.get().is_none() {
        return;
    }
    let Some(id) = test_state::current_id() else {
        return;
    };
    let prev = test_state::set_signed_out_for(id, signed_out);
    if prev != signed_out {
        log::info!("[scheduler_gate] signed_out {} -> {}", prev, signed_out);
        // #2831: mirror the production sign-in wake so tests exercise the
        // same resume-notify path (true -> false fires the loop wake).
        if prev && !signed_out {
            resume_notify().notify_one();
        }
    }
}

/// Test-only RAII helper that snapshots the per-runtime `signed_out`
/// flag on construction, flips it to `next`, and restores the
/// snapshotted value on drop — even if the test body panics.
///
/// Use this in any test that exercises a code path that itself calls
/// [`set_signed_out`] *after* [`init_global`] has promoted [`STATE`].
/// Notably the JSON-RPC server bootstrap (`run_server_embedded` →
/// `bootstrap_core_runtime` → `register_domain_subscribers`) flips
/// the flag to `true` whenever the workspace has no stored session
/// token, which is the common case for tests using a fresh
/// `tempfile::tempdir()` workspace.
///
/// Bypasses the writer-side gate at [`set_signed_out`] (which no-ops
/// only when `STATE` is `None`) so it works regardless of whether
/// `init_global` has run.
#[cfg(test)]
pub(crate) struct SignedOutTestGuard(Option<(tokio::runtime::Id, bool)>);

#[cfg(test)]
impl SignedOutTestGuard {
    /// Snapshot the per-runtime `signed_out` flag, write `next`, and
    /// return a guard that restores the snapshotted value on drop.
    /// No-op outside a tokio runtime.
    pub(crate) fn set(next: bool) -> Self {
        match test_state::current_id() {
            Some(id) => {
                let prev = test_state::set_signed_out_for(id, next);
                Self(Some((id, prev)))
            }
            None => Self(None),
        }
    }
}

#[cfg(test)]
impl Drop for SignedOutTestGuard {
    fn drop(&mut self) {
        if let Some((id, prev)) = self.0 {
            test_state::set_signed_out_for(id, prev);
        }
    }
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

/// Cooperatively block a caller until the host is ready for LLM-bound
/// work, then hand back an [`LlmPermit`] that holds a slot in the global
/// LLM semaphore.
///
/// Policy-driven backoff happens **before** semaphore acquisition so a
/// `Paused` mode doesn't pile up tasks queued for the slot — they sit
/// in the pause-poll loop, not in the semaphore wait queue.
///
/// * **Aggressive / Normal** — wait for the global slot; return immediately
///   once granted.
/// * **Throttled** — sleep `throttled_backoff_ms` first so concurrent
///   workers serialise themselves, then acquire the slot.
/// * **Paused** — poll every `paused_poll_ms` until the policy changes,
///   then acquire the slot.
///
/// Drop the returned [`LlmPermit`] as soon as the LLM call returns.
///
/// Returns `None` only if the global LLM semaphore has been closed
/// (never happens in production — the semaphore lives for the lifetime
/// of the process). Callers can safely treat `None` as "skip the
/// gate" rather than propagating an error.
pub async fn wait_for_capacity() -> Option<LlmPermit> {
    loop {
        // Signed-out override is checked first and uses the same paused-poll
        // cadence as the rest of the Paused arm. Holding here (rather than
        // returning) means workers naturally resume the instant the user
        // signs back in — no respawn dance, no missed wakeups.
        //
        // We gate on `STATE.get().is_some()` so the override only fires once
        // the gate has been initialised by `init_global`. In unit tests
        // where `init_global` was never called there is no background-worker
        // pool to stand down, but the per-runtime `signed_out` flag can
        // still be `true` from an earlier test that exercised the credentials
        // / 401 paths (`clear_session`, RPC 401 dispatch, or
        // `SessionExpiredSubscriber.handle()`). Without the gate, every
        // subsequent caller of `wait_for_capacity` polls forever on the
        // 60-second fallback cadence — manifest as the
        // `openhuman::agent::triage::evaluator::tests::*` hangs reported
        // after #1516.
        if STATE.get().is_some() && is_signed_out() {
            let paused_ms = STATE
                .get()
                .map(|s| s.read().cfg.paused_poll_ms)
                .unwrap_or(60_000);
            log::trace!("[scheduler_gate] paused (signed_out); polling every {paused_ms}ms");
            tokio::time::sleep(Duration::from_millis(paused_ms)).await;
            continue;
        }

        let (policy, throttled_ms, paused_ms) = match STATE.get() {
            Some(state) => {
                let g = state.read();
                (g.policy, g.cfg.throttled_backoff_ms, g.cfg.paused_poll_ms)
            }
            None => {
                // Gate not initialised (unit tests, early bootstrap).
                // Acquire directly — no policy to consult.
                return acquire_llm_permit_inner().await;
            }
        };
        match policy {
            Policy::Aggressive | Policy::Normal => {
                return acquire_llm_permit_inner().await;
            }
            Policy::Throttled => {
                log::trace!(
                    "[scheduler_gate] throttled — sleeping {throttled_ms}ms before permit acquire"
                );
                tokio::time::sleep(Duration::from_millis(throttled_ms)).await;
                return acquire_llm_permit_inner().await;
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

async fn acquire_llm_permit_inner() -> Option<LlmPermit> {
    let sem = llm_permits();
    match sem.acquire_owned().await {
        Ok(permit) => {
            log::trace!("[scheduler_gate] llm permit acquired");
            Some(LlmPermit { _permit: permit })
        }
        Err(_) => {
            // Semaphore closed — should never happen since we never
            // close it. Log loudly and let the caller proceed without
            // a permit so the pipeline doesn't deadlock.
            log::warn!(
                "[scheduler_gate] llm semaphore closed unexpectedly — proceeding without a permit"
            );
            None
        }
    }
}

/// Test/diagnostic hook: try to grab a permit without consulting the
/// gate policy. Returns `None` if no slots are free. **Do not** call
/// from production code — production callers should use
/// [`wait_for_capacity`] so the policy backoff applies.
#[cfg(test)]
pub fn try_acquire_llm_permit() -> Option<LlmPermit> {
    let sem = llm_permits();
    sem.try_acquire_owned()
        .ok()
        .map(|p| LlmPermit { _permit: p })
}

/// Number of permits currently available. Test-only diagnostic.
#[cfg(test)]
pub fn available_llm_permits() -> usize {
    llm_permits().available_permits()
}

#[cfg(test)]
#[path = "gate_tests.rs"]
mod tests;
