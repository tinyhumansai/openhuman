use super::*;
use crate::config::schema::SchedulerGateMode;
use std::sync::atomic::{AtomicBool, Ordering};

fn calm() -> Signals {
    Signals {
        on_ac_power: true,
        battery_charge: None,
        cpu_usage_pct: 5.0,
        server_mode: false,
    }
}

fn busy() -> Signals {
    Signals {
        cpu_usage_pct: 75.0,
        ..calm()
    }
}

fn cfg(mode: SchedulerGateMode) -> SchedulerGateConfig {
    SchedulerGateConfig {
        mode,
        throttled_backoff_ms: 1_000,
        paused_poll_ms: 500,
        ..Default::default()
    }
}

fn core(mode: SchedulerGateMode, signals: Signals) -> SharedCore {
    Arc::new(RwLock::new(GateCore::new(cfg(mode), signals)))
}

fn never() -> bool {
    false
}

#[tokio::test]
async fn uninitialised_gate_hands_back_a_permit_and_releases_it_on_drop() {
    let slots = new_llm_slots();
    let permit = wait_for_capacity(None, never, &slots).await;
    assert!(permit.is_some());
    assert_eq!(
        slots.available_permits(),
        0,
        "permit occupies the only slot"
    );
    drop(permit);
    assert_eq!(slots.available_permits(), LLM_SLOTS);
}

#[tokio::test]
async fn signed_out_is_ignored_until_a_gate_exists() {
    let slots = new_llm_slots();
    let permit = tokio::time::timeout(
        Duration::from_millis(500),
        wait_for_capacity(None, || true, &slots),
    )
    .await
    .expect("an uninitialised gate must not block on the signed-out flag");
    assert!(permit.is_some());
}

#[tokio::test]
async fn the_semaphore_holds_exactly_one_slot() {
    let slots = new_llm_slots();
    let first = wait_for_capacity(None, never, &slots).await.unwrap();
    assert!(try_acquire_llm_permit(&slots).is_none());
    drop(first);
    assert!(try_acquire_llm_permit(&slots).is_some());
}

#[tokio::test(start_paused = true)]
async fn a_second_waiter_blocks_until_the_first_drops() {
    let slots = new_llm_slots();
    let first = wait_for_capacity(None, never, &slots).await.unwrap();
    let waiter = {
        let slots = slots.clone();
        tokio::spawn(async move { wait_for_capacity(None, never, &slots).await })
    };
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert!(!waiter.is_finished());
    drop(first);
    assert!(waiter.await.unwrap().is_some());
}

#[tokio::test(start_paused = true)]
async fn normal_policy_does_not_sleep() {
    let core = core(SchedulerGateMode::Auto, calm());
    assert_eq!(core.read().policy(), Policy::Normal);
    let slots = new_llm_slots();
    let started = tokio::time::Instant::now();
    let permit = wait_for_capacity(Some(&core), never, &slots).await;
    assert!(permit.is_some());
    assert_eq!(started.elapsed(), Duration::ZERO);
}

#[tokio::test(start_paused = true)]
async fn throttled_policy_sleeps_the_backoff_before_acquiring() {
    let core = core(SchedulerGateMode::Auto, busy());
    assert_eq!(core.read().policy(), Policy::Throttled);
    let slots = new_llm_slots();
    let started = tokio::time::Instant::now();
    let permit = wait_for_capacity(Some(&core), never, &slots).await;
    assert!(permit.is_some());
    assert_eq!(started.elapsed(), Duration::from_millis(1_000));
}

#[tokio::test(start_paused = true)]
async fn paused_policy_polls_until_the_user_turns_the_gate_back_on() {
    let core = core(SchedulerGateMode::Off, calm());
    assert!(matches!(core.read().policy(), Policy::Paused { .. }));
    let slots = new_llm_slots();
    let waiter = {
        let core = core.clone();
        let slots = slots.clone();
        tokio::spawn(async move { wait_for_capacity(Some(&core), never, &slots).await })
    };
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    assert!(!waiter.is_finished(), "still paused, so still waiting");
    assert!(core.write().update_config(cfg(SchedulerGateMode::AlwaysOn)));
    assert!(waiter.await.unwrap().is_some());
}

#[tokio::test(start_paused = true)]
async fn the_signed_out_override_holds_callers_until_it_clears() {
    let core = core(SchedulerGateMode::Auto, calm());
    let slots = new_llm_slots();
    let signed_out = Arc::new(AtomicBool::new(true));
    let waiter = {
        let core = core.clone();
        let slots = slots.clone();
        let flag = signed_out.clone();
        tokio::spawn(async move {
            wait_for_capacity(Some(&core), move || flag.load(Ordering::Acquire), &slots).await
        })
    };
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    assert!(!waiter.is_finished());
    signed_out.store(false, Ordering::Release);
    assert!(waiter.await.unwrap().is_some());
}

#[test]
fn update_config_reports_only_a_transition_out_of_paused() {
    let mut core = GateCore::new(cfg(SchedulerGateMode::Off), calm());
    assert!(
        !core.update_config(cfg(SchedulerGateMode::Off)),
        "paused to paused is not a resume"
    );
    assert!(core.update_config(cfg(SchedulerGateMode::AlwaysOn)));
    assert!(
        !core.update_config(cfg(SchedulerGateMode::AlwaysOn)),
        "running to running is not a resume"
    );
    assert_eq!(core.config().mode, SchedulerGateMode::AlwaysOn);
}

#[test]
fn refresh_recomputes_the_policy_from_new_signals() {
    let mut core = GateCore::new(cfg(SchedulerGateMode::Auto), calm());
    assert_eq!(core.policy(), Policy::Normal);
    core.refresh(busy());
    assert_eq!(core.policy(), Policy::Throttled);
    assert_eq!(core.signals().cpu_usage_pct, 75.0);
    core.refresh(calm());
    assert_eq!(core.policy(), Policy::Normal);
}
