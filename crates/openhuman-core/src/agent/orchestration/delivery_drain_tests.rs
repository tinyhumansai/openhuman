use super::*;
use crate::agent::orchestration::background_completions::{
    pending_for, record_completion, router_for_workspace, TestWorkspace,
};
use crate::agent::orchestration::busy_guard::TurnBusy;
use crate::storage::fence::{system_now_ms, ScopeMatch};
use crate::storage::lease::{DocumentLeases, LeaseStore};
use crate::storage::MemoryStorage;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

const TTL: Duration = Duration::from_secs(30);
const SHORT: Duration = Duration::from_millis(20);

/// A profile's fence for a grant that is good for the TTL, plus the lease
/// store it verifies against (kept alive by the caller).
fn live_fence() -> (Arc<dyn LeaseStore>, Arc<LeaseFence>) {
    let backend = MemoryStorage::new();
    let leases: Arc<dyn LeaseStore> =
        Arc::new(DocumentLeases::cluster(&backend, "node-a", None, TTL).unwrap());
    let fence = Arc::new(LeaseFence::new(
        Arc::clone(&leases),
        "alice",
        "node-a",
        1,
        system_now_ms() + 30_000,
        5_000,
        vec![ScopeMatch::Exact("profile:alice".into())],
    ));
    (leases, fence)
}

/// Run a drain whose `deliver` only notes that it ran.
async fn drain(
    thread: &str,
    delay: Duration,
    wait_idle: bool,
    fence: &DrainFence,
) -> (DrainOutcome, bool) {
    let ran = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&ran);
    let outcome = run_drain(thread, delay, wait_idle, fence, SHORT, || async move {
        flag.store(true, Ordering::SeqCst);
    })
    .await;
    (outcome, ran.load(Ordering::SeqCst))
}

#[tokio::test]
async fn a_drain_for_a_fenced_profile_delivers_nothing() {
    let (_leases, fence) = live_fence();
    fence.fence();
    let (outcome, ran) = drain("dd-fenced", SHORT, true, &DrainFence::of(fence)).await;
    assert_eq!(outcome, DrainOutcome::Fenced);
    assert!(!ran, "a node that lost the lease must not deliver");
}

#[tokio::test]
async fn a_drain_for_a_live_profile_still_delivers() {
    let (_leases, fence) = live_fence();
    let (outcome, ran) = drain("dd-live", SHORT, true, &DrainFence::of(fence)).await;
    assert_eq!(outcome, DrainOutcome::Ran);
    assert!(ran);
    let (outcome, ran) = drain("dd-unfenced", SHORT, false, &DrainFence::none()).await;
    assert_eq!(outcome, DrainOutcome::Ran, "no fence (desktop) always runs");
    assert!(ran);
}

#[tokio::test]
async fn losing_the_lease_during_the_delay_cuts_the_drain_short() {
    let (_leases, fence) = live_fence();
    let latch = Arc::clone(&fence);
    tokio::spawn(async move {
        tokio::time::sleep(SHORT).await;
        latch.fence();
    });
    let started = std::time::Instant::now();
    let (outcome, ran) = drain(
        "dd-mid-delay",
        Duration::from_secs(60),
        false,
        &DrainFence::of(fence),
    )
    .await;
    assert_eq!(outcome, DrainOutcome::Fenced);
    assert!(!ran);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "aborted, not slept out"
    );
}

#[tokio::test]
async fn an_expired_grant_fences_the_drain_without_a_latch() {
    let backend = MemoryStorage::new();
    let leases: Arc<dyn LeaseStore> =
        Arc::new(DocumentLeases::cluster(&backend, "node-a", None, TTL).unwrap());
    // Renewals stopped landing: the grant ran out by this node's clock.
    let fence = Arc::new(LeaseFence::new(
        Arc::clone(&leases),
        "alice",
        "node-a",
        1,
        system_now_ms().saturating_sub(1),
        0,
        vec![],
    ));
    let (outcome, ran) = drain("dd-expired", SHORT, false, &DrainFence::of(fence)).await;
    assert_eq!(outcome, DrainOutcome::Fenced);
    assert!(!ran);
}

#[tokio::test]
async fn losing_the_lease_while_waiting_for_an_idle_thread_drops_the_drain() {
    let _g = crate::config::TEST_ENV_LOCK.lock().await;
    let (_leases, fence) = live_fence();
    let turn = TurnBusy::start_on_thread("dd-busy-fenced");
    let latch = Arc::clone(&fence);
    tokio::spawn(async move {
        tokio::time::sleep(SHORT * 3).await;
        latch.fence();
    });
    let (outcome, ran) = drain("dd-busy-fenced", SHORT, true, &DrainFence::of(fence)).await;
    drop(turn);
    assert_eq!(outcome, DrainOutcome::Fenced);
    assert!(!ran);
}

#[tokio::test]
async fn a_recovered_drain_waits_for_a_relayed_turn_on_its_thread() {
    let _g = crate::config::TEST_ENV_LOCK.lock().await;
    let thread = "dd-relayed-thread";
    let turn = TurnBusy::start_on_thread(thread);
    assert!(is_busy(thread), "a relayed turn marks its thread busy");
    let release = tokio::spawn(async move {
        tokio::time::sleep(SHORT * 3).await;
        drop(turn);
    });
    let (outcome, ran) = drain(thread, SHORT, true, &DrainFence::none()).await;
    release.await.unwrap();
    assert_eq!(outcome, DrainOutcome::Ran);
    assert!(ran);
    assert!(!is_busy(thread));
}

#[tokio::test]
async fn a_delivery_waits_while_a_relayed_turn_runs_on_its_thread() {
    let _g = crate::config::TEST_ENV_LOCK.lock().await;
    let ws = TestWorkspace::new();
    let w = ws.path();
    let thread = "dd-relay-deliver";
    record_completion(
        w,
        "dd-relay-session",
        "sub-relay",
        "researcher",
        "finished during the relayed turn",
        Some(thread.to_owned()),
    )
    .await;
    let router = router_for_workspace(w);
    let turns = Arc::new(AtomicU32::new(0));
    let attempt = |turns: Arc<AtomicU32>| {
        super::super::background_delivery::try_deliver_for_test(
            thread.to_string(),
            Arc::clone(&router),
            turns,
        )
    };

    let relayed = TurnBusy::start_on_thread(thread);
    attempt(Arc::clone(&turns)).await;
    assert_eq!(
        turns.load(Ordering::SeqCst),
        0,
        "no delivery over a running relayed turn"
    );
    assert_eq!(pending_for(w, thread).len(), 1, "the result stays pending");

    drop(relayed);
    attempt(Arc::clone(&turns)).await;
    assert_eq!(
        turns.load(Ordering::SeqCst),
        1,
        "delivered once the turn ends"
    );
    assert!(pending_for(w, thread).is_empty());
}
