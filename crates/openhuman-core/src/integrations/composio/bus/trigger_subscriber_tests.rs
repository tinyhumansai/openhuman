//! Regression tests for routing Composio triggers.
//!
//! The triggers run against the real `tinyeventqueues` module, linked into
//! the test binary on a private bus (`crate::inbound::fixture`), with triage
//! scripted. Before the inbound event queues, a trigger whose triage was
//! `Deferred` was logged and dropped, and a redelivered trigger was triaged
//! (and acted on) once per delivery.

use std::time::{Duration, Instant};

use serde_json::json;

use super::*;
use crate::inbound::fixture::{active_config, decision, deferred, Rig, ScriptedTriage};
use crate::triage::TriageAction;

const LABEL: &str = "composio/gmail/GMAIL_NEW_GMAIL_MESSAGE";

fn trigger(uuid: &str) -> TriggerEnvelope {
    TriggerEnvelope::from_composio(
        "gmail",
        "GMAIL_NEW_GMAIL_MESSAGE",
        "msg-id",
        uuid,
        json!({ "subject": "quarterly numbers", "from": "a@example.com" }),
    )
}

async fn route(rig: &Rig, config: Option<crate::config::Config>, envelope: TriggerEnvelope) {
    let routed = route_trigger(
        DispatchCtx::host_default(),
        config,
        envelope,
        rig.triage.clone(),
        Some(rig.client.clone()),
    )
    .await;
    if let Routed::Direct(task) = routed {
        task.await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_deferred_composio_trigger_is_retried_at_its_defer_time_instead_of_dropped() {
    let triage = ScriptedTriage::new([deferred(400), decision(TriageAction::React)]);
    let rig = Rig::new(triage.clone()).await;
    let started = Instant::now();

    route(&rig, Some(active_config()), trigger("uuid-deferred")).await;

    let applied = triage.until_applied(1, Duration::from_secs(15)).await;
    assert_eq!(
        applied,
        vec![LABEL.to_owned()],
        "the deferred trigger was dropped"
    );
    assert_eq!(triage.runs(), 2, "triaged, deferred, then triaged again");
    assert!(
        started.elapsed() >= Duration::from_millis(350),
        "retried before the deferral ran out: {:?}",
        started.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_redelivered_composio_trigger_produces_one_turn() {
    let triage = ScriptedTriage::new([decision(TriageAction::React)]);
    let rig = Rig::new(triage.clone()).await;

    // The backend redelivers the same trigger (same uuid) three times.
    for _ in 0..3 {
        route(&rig, Some(active_config()), trigger("uuid-redelivered")).await;
    }

    let applied = triage.until_applied(1, Duration::from_secs(15)).await;
    assert_eq!(applied, vec![LABEL.to_owned()]);
    // Give a second turn every chance to appear.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        triage.applied().len(),
        1,
        "a redelivery produced another turn"
    );
    assert_eq!(triage.runs(), 1, "a redelivery was triaged again");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_the_queues_off_a_trigger_is_triaged_directly() {
    let triage = ScriptedTriage::new([decision(TriageAction::React)]);
    let rig = Rig::new(triage.clone()).await;
    let mut config = active_config();
    config.inbound.enabled = false;

    route(&rig, Some(config), trigger("uuid-direct")).await;

    assert_eq!(triage.applied(), vec![LABEL.to_owned()]);
    assert_eq!(triage.runs(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_config_a_trigger_is_triaged_directly() {
    let triage = ScriptedTriage::new(Vec::<crate::inbound::fixture::Step>::new());
    let rig = Rig::new(triage.clone()).await;

    route(&rig, None, trigger("uuid-no-config")).await;

    assert_eq!(triage.runs(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_direct_path_logs_and_drops_a_deferral_and_a_terminal_outcome() {
    let triage = ScriptedTriage::new([
        deferred(10),
        crate::triage::TriageOutcome::Terminal {
            reason: "no fallback".into(),
        }
        .into(),
    ]);
    for uuid in ["uuid-d1", "uuid-d2"] {
        triage_directly(DispatchCtx::host_default(), trigger(uuid), triage.clone())
            .await
            .unwrap();
    }
    assert_eq!(triage.runs(), 2);
    assert!(triage.applied().is_empty());
}

#[test]
fn the_subscriber_names_itself_and_listens_to_composio() {
    let subscriber = ComposioTriggerSubscriber::new();
    assert_eq!(subscriber.name(), "composio::trigger");
    assert_eq!(subscriber.domains(), Some(&["composio"][..]));
}
