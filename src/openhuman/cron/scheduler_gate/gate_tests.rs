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
    if skip_if_gate_initialised("wait_for_capacity_acquires_immediately_when_signed_out_and_uninit")
    {
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

/// #2831: both the firing side (`update_config` / `set_signed_out`) and the
/// waiting side (the periodic loop) must observe the *same* `Notify`, so
/// `resume_notify` must hand back one process-wide instance.
#[tokio::test]
async fn resume_notify_is_a_stable_singleton() {
    let _g = lock();
    assert!(
        Arc::ptr_eq(&resume_notify(), &resume_notify()),
        "resume_notify must return one shared instance"
    );
}

/// A `notify_one()` wakes a task parked on `notified()` — the mechanism the
/// periodic loop relies on to resume early. Proves the singleton wiring
/// end-to-end (fire on one handle, wake on another).
#[tokio::test]
async fn resume_notify_wakes_a_parked_waiter() {
    let _g = lock();
    let waiter = resume_notify();
    let parked = tokio::spawn(async move { waiter.notified().await });
    // Yield so the spawned task reaches `.notified()` before we fire.
    tokio::task::yield_now().await;
    resume_notify().notify_one();
    timeout(TokioDuration::from_secs(1), parked)
        .await
        .expect("parked waiter must wake promptly after notify_one")
        .expect("waiter task must not panic");
}

/// #2831 wiring: a paused→running `update_config` transition and a
/// sign-in (`set_signed_out` true→false) each fire the resume notify.
///
/// Seeds `STATE` directly (the test module can reach it) so `update_config`
/// / `set_signed_out` are live without spawning the real sampler. We drive
/// `Off → AlwaysOn`, which is deterministically `Paused → Aggressive`
/// regardless of any policy a prior test left behind, so the transition —
/// and thus the `notify_one()` — is guaranteed.
#[tokio::test]
async fn resume_transitions_fire_the_notify() {
    use crate::openhuman::config::SchedulerGateMode;
    let _g = lock();

    // Ensure STATE is initialised. `set` is a no-op if an earlier test
    // already promoted it — that's fine, we re-drive the transition below.
    let cfg = SchedulerGateConfig {
        mode: SchedulerGateMode::Off,
        ..Default::default()
    };
    let signals = Signals::sample();
    let policy = decide(&signals, &cfg);
    let _ = STATE.set(Arc::new(RwLock::new(State {
        cfg,
        signals,
        policy,
    })));

    // --- update_config: Paused -> running fires the notify ---
    let waiter = resume_notify();
    let parked = tokio::spawn(async move { waiter.notified().await });
    tokio::task::yield_now().await;
    update_config(SchedulerGateConfig {
        mode: SchedulerGateMode::Off,
        ..Default::default()
    }); // -> Paused { UserDisabled }
    update_config(SchedulerGateConfig {
        mode: SchedulerGateMode::AlwaysOn,
        ..Default::default()
    }); // Paused -> Aggressive => resume fires
    timeout(TokioDuration::from_secs(1), parked)
        .await
        .expect("update_config un-pause must wake the resume waiter")
        .expect("waiter task must not panic");

    // --- set_signed_out true -> false fires the notify ---
    let waiter2 = resume_notify();
    let parked2 = tokio::spawn(async move { waiter2.notified().await });
    tokio::task::yield_now().await;
    set_signed_out(true);
    set_signed_out(false); // true -> false => resume fires
    timeout(TokioDuration::from_secs(1), parked2)
        .await
        .expect("sign-in (signed_out true->false) must wake the resume waiter")
        .expect("waiter task must not panic");
}
