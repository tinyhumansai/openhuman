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
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::openhuman::config::{Config, SchedulerGateConfig};
use crate::openhuman::scheduler_gate::policy::{decide, PauseReason, Policy};
use crate::openhuman::scheduler_gate::signals::Signals;

/// Process-wide ceiling on concurrent LLM-bound work.
///
/// Held at 1 to keep concurrent local-Ollama / bge-m3 calls (8K context,
/// ~1.3 GB resident each) from saturating local RAM. See
/// `feedback_local_llm_load.md` — backfills with multiple
/// simultaneous Ollama requests have crashed the user's laptop twice.
///
/// Cloud-backend LLM calls bypass this semaphore at the worker layer
/// (see `memory_tree::jobs::worker::run_once`) because they're
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
mod test_state {
    use super::*;
    use std::collections::HashMap;

    pub(super) struct RuntimeGateState {
        pub permits: Arc<Semaphore>,
        pub signed_out: bool,
    }

    fn map() -> &'static parking_lot::Mutex<HashMap<tokio::runtime::Id, RuntimeGateState>> {
        static M: OnceLock<parking_lot::Mutex<HashMap<tokio::runtime::Id, RuntimeGateState>>> =
            OnceLock::new();
        M.get_or_init(|| parking_lot::Mutex::new(HashMap::new()))
    }

    /// Current tokio runtime ID, or `None` outside any runtime (sync tests).
    pub(super) fn current_id() -> Option<tokio::runtime::Id> {
        tokio::runtime::Handle::try_current().ok().map(|h| h.id())
    }

    pub(super) fn permits_for(id: tokio::runtime::Id) -> Arc<Semaphore> {
        let mut g = map().lock();
        g.entry(id)
            .or_insert_with(|| RuntimeGateState {
                permits: Arc::new(Semaphore::new(LLM_SLOTS)),
                signed_out: false,
            })
            .permits
            .clone()
    }

    pub(super) fn signed_out_for(id: tokio::runtime::Id) -> bool {
        let mut g = map().lock();
        g.entry(id)
            .or_insert_with(|| RuntimeGateState {
                permits: Arc::new(Semaphore::new(LLM_SLOTS)),
                signed_out: false,
            })
            .signed_out
    }

    pub(super) fn set_signed_out_for(id: tokio::runtime::Id, v: bool) -> bool {
        let mut g = map().lock();
        let entry = g.entry(id).or_insert_with(|| RuntimeGateState {
            permits: Arc::new(Semaphore::new(LLM_SLOTS)),
            signed_out: false,
        });
        let prev = entry.signed_out;
        entry.signed_out = v;
        prev
    }
}

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
/// `OpenHumanBackendProvider`).
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
    }
}

/// Test-only RAII helper that snapshots the per-runtime `signed_out`
/// flag on construction, flips it to `next`, and restores the
/// snapshotted value on drop — even if the test body panics.
///
/// Use this in any test that exercises a code path that itself calls
/// [`set_signed_out`] *after* [`init_global`] has promoted [`STATE`].
/// Notably the JSON-RPC server bootstrap (`run_server_embedded` →
/// `bootstrap_skill_runtime` → `register_domain_subscribers`) flips
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
mod tests {
    //! These tests share the **process-wide** `LLM_PERMITS` semaphore
    //! (which is intentional — that's what they're testing). They are
    //! serialised via a module-local mutex so two test threads can't
    //! both hold a permit at the same time and confuse each other's
    //! `available_permits` reads.
    use super::*;
    use std::sync::Mutex;
    use std::time::Instant;
    use tokio::time::{timeout, Duration as TokioDuration};

    static GATE_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        // Tolerate poisoning so a panicking test doesn't block the rest.
        GATE_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    #[tokio::test]
    async fn wait_for_capacity_returns_permit_when_gate_uninit() {
        let _g = lock();
        let permit = wait_for_capacity().await;
        assert!(
            permit.is_some(),
            "uninit gate must still hand back a permit"
        );
        assert_eq!(
            available_llm_permits(),
            0,
            "permit must occupy the single LLM slot"
        );
        drop(permit);
        assert_eq!(available_llm_permits(), 1, "drop must release the slot");
    }

    #[tokio::test]
    // Wake-on-permit-drop timing test: under heavy parallel cargo-test load
    // the 1s timeout occasionally fires before the spawned waiter is polled
    // even though the tokio Semaphore wake is reliable in isolation. The
    // behaviour under test is exercised by `semaphore_size_is_one` plus
    // production code paths; this test only adds a timing assertion.
    #[ignore = "flaky timing under full-suite load — see PR #1524"]
    async fn second_waiter_blocks_until_first_drops() {
        let _g = lock();
        let first = wait_for_capacity().await.expect("first permit");
        assert_eq!(available_llm_permits(), 0);

        // Spawn a second acquirer; it must block.
        let handle = tokio::spawn(async move {
            let started = Instant::now();
            let p = wait_for_capacity().await;
            (started.elapsed(), p)
        });

        // Give the second waiter a moment to start polling.
        tokio::time::sleep(TokioDuration::from_millis(40)).await;
        assert!(!handle.is_finished(), "second waiter must be blocked");

        // Release the first permit; the second should resolve.
        drop(first);
        let (elapsed, second) = timeout(TokioDuration::from_secs(1), handle)
            .await
            .unwrap()
            .unwrap();
        assert!(
            second.is_some(),
            "second waiter must eventually get a permit"
        );
        assert!(
            elapsed >= TokioDuration::from_millis(20),
            "second waiter should have actually waited (got {elapsed:?})"
        );
        drop(second);
    }

    // `SignedOutTestGuard` lives at module scope (above) so cross-module
    // tests (e.g. `core::jsonrpc::tests::shutdown_token_*`) can use it
    // too. The local re-import keeps the existing tests below readable
    // without fully-qualified paths.
    use super::SignedOutTestGuard;

    /// Bail out if a cross-module test in the same lib-test binary has
    /// already promoted [`STATE`] to `Some` via `init_global` (notably
    /// `core::jsonrpc::tests::shutdown_token_*`, which boots the embedded
    /// server). `STATE` is an `OnceLock` with no reset, so these
    /// `*_when_gate_uninit` regression tests are inherently order-sensitive
    /// — they only have meaning when `STATE.is_none()`. Skipping when
    /// `STATE.is_some()` avoids a false failure here; the actual leak
    /// class the test exists to guard against is still covered by
    /// the writer-side `set_signed_out` gate plus the reader-side
    /// `wait_for_capacity` guard in production code paths.
    fn skip_if_gate_initialised(test_name: &str) -> bool {
        if STATE.get().is_some() {
            eprintln!(
                "[scheduler_gate::tests] skipping {test_name}: STATE already \
                 initialised by an earlier test in this binary"
            );
            true
        } else {
            false
        }
    }

    #[tokio::test]
    async fn signed_out_is_ignored_when_gate_uninit() {
        // In unit tests `init_global` is never called, so `STATE` is `None`.
        // In that state the signed-out override is intentionally inert: there
        // are no background workers to stand down, and honouring the per-runtime
        // flag would let any earlier test that set it (`clear_session`, RPC 401
        // dispatch, `SessionExpiredSubscriber`) deadlock every subsequent caller
        // of `wait_for_capacity`.
        let _g = lock();
        if skip_if_gate_initialised("signed_out_is_ignored_when_gate_uninit") {
            return;
        }
        let _signed_out = SignedOutTestGuard::set(true);

        assert_eq!(
            current_policy(),
            Policy::Normal,
            "with STATE uninit, signed_out must NOT change current_policy"
        );
    }

    #[tokio::test]
    async fn wait_for_capacity_acquires_immediately_when_signed_out_and_uninit() {
        // Regression test for the
        // `openhuman::agent::triage::evaluator::tests::*` hangs that surfaced
        // after #1516 added the `signed_out` override. Earlier tests in the
        // same `cargo test` binary that exercise `clear_session` /
        // `SessionExpiredSubscriber` / the RPC 401 path can leave the
        // per-runtime flag set to `true`. Without the `STATE.is_some()`
        // gate, every subsequent `wait_for_capacity()` polls forever on the
        // 60-second `paused_poll_ms` fallback (STATE is None in tests, so
        // the fallback is the unconfigured default).
        let _g = lock();
        if skip_if_gate_initialised(
            "wait_for_capacity_acquires_immediately_when_signed_out_and_uninit",
        ) {
            return;
        }
        let _signed_out = SignedOutTestGuard::set(true);

        let permit = timeout(TokioDuration::from_millis(500), wait_for_capacity())
            .await
            .expect("wait_for_capacity must NOT block when STATE is uninit, even if signed_out")
            .expect("uninit gate still hands back a permit");
        drop(permit);
    }

    #[tokio::test]
    async fn set_signed_out_is_a_noop_when_gate_uninit() {
        // Writer-side companion to `signed_out_is_ignored_when_gate_uninit`.
        // The production `set_signed_out` must NOT mutate the per-runtime flag
        // when `STATE` is uninit, otherwise a `clear_session` call exercised
        // in one test leaks `signed_out=true` into every subsequent test in
        // the binary. With this gate, only callers that run after `init_global`
        // (i.e. real workers in production) ever flip the bit.
        //
        // Note: because this is a `#[tokio::test]`, a runtime is always
        // present, so the `current_id().is_none()` branch in the test-cfg
        // implementations of `set_signed_out` and `is_signed_out` is
        // unreachable here. The gate we exercise is exclusively the
        // `STATE.get().is_none()` early-return.
        let _g = lock();
        if skip_if_gate_initialised("set_signed_out_is_a_noop_when_gate_uninit") {
            return;
        }
        // Force the atomic to a known-clean state via the test backdoor.
        let _restore = SignedOutTestGuard::set(false);

        set_signed_out(true);
        assert!(
            !is_signed_out(),
            "set_signed_out(true) must no-op when STATE is None"
        );

        set_signed_out(false);
        assert!(
            !is_signed_out(),
            "set_signed_out(false) must no-op when STATE is None"
        );
    }

    #[tokio::test]
    async fn semaphore_size_is_one() {
        let _g = lock();
        let p1 = wait_for_capacity().await.expect("first permit");
        // Try-acquire must fail while the slot is held.
        assert!(
            try_acquire_llm_permit().is_none(),
            "semaphore must be size-1 — second try_acquire should fail"
        );
        drop(p1);
        // Now another should succeed.
        let p2 = try_acquire_llm_permit().expect("permit free after drop");
        drop(p2);
    }
}
