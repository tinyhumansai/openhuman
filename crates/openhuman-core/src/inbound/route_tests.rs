//! Tests for the hand-off to the queues.

use std::time::Duration;

use serde_json::json;

use super::*;
use crate::inbound::claims::Delivery;
use crate::inbound::events;
use crate::inbound::fixture::{active_config, Rig, ScriptedTriage, Step};
use crate::triage::{TriageOutcome, TriggerEnvelope};

fn event(uuid: &str) -> InboundEvent {
    let envelope = TriggerEnvelope::from_composio("slack", "SLACK_MESSAGE", "", uuid, json!({}));
    events::composio(&envelope).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_accepted_event_is_queued_and_a_redelivery_ends_at_ingress() {
    let rig = Rig::new(ScriptedTriage::new(Vec::<Step>::new())).await;
    let config = active_config();
    let first = offer(&config, &rig.client, event("u1")).await;
    assert_eq!(first, Offered::Queued(Disposition::Accepted));
    assert!(first.is_queued());
    let again = offer(&config, &rig.client, event("u1")).await;
    assert_eq!(again, Offered::Queued(Disposition::Duplicate));
    let _ = rig.triage.until_applied(1, Duration::from_secs(5)).await;
}

#[tokio::test]
async fn inactive_queues_send_the_event_down_the_direct_path() {
    let mut config = active_config();
    config.inbound.enabled = false;
    let client = crate::modules::ops::client(config.clone());
    let offered = offer(&config, &client, event("u2")).await;
    assert!(matches!(offered, Offered::Direct(ref why) if why.contains("inactive")));
    assert!(!offered.is_queued());
}

/// A client bound to an object nobody serves: every call fails without
/// proving the module never saw the event, so each attempt is ambiguous.
/// The connection is returned with it to keep the bus alive.
async fn ambiguous_client() -> (ModuleClient, tinybus::Connection) {
    let bus = tinybus::transport::memory::MemoryBus::new();
    tinybus::broker::Broker::new().spawn(bus.clone());
    let caller = tinybus::Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let proxy = caller
        .proxy(
            tinyeventqueues_bus::INTERFACE,
            tinyeventqueues_bus::OBJECT_PATH,
            tinyeventqueues_bus::INTERFACE,
        )
        .unwrap();
    (crate::modules::ops::fixture_client(proxy), caller)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_module_sends_the_event_down_the_direct_path() {
    let (client, _bus) = ambiguous_client().await;
    let offered = offer(&active_config(), &client, event("u3")).await;
    assert!(matches!(offered, Offered::Direct(ref why) if why.contains("ingest failed")));
}

/// The caller's direct path, as the Composio and webhook paths run it.
async fn run_directly(triage: &ScriptedTriage, event: &InboundEvent) {
    use crate::triage::TriageRunner;
    let envelope = events::envelope_of(event).unwrap();
    let ctx = crate::core::runtime::DispatchCtx::host_default();
    let TriageOutcome::Decision(run) = triage.run(&ctx, &envelope).await.unwrap() else {
        panic!("the scripted triage decides");
    };
    triage.apply(&ctx, run, &envelope).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_timed_out_ingest_that_landed_is_triaged_once() {
    // #8154: every attempt is ambiguous, so the caller triages the event
    // directly; the write had landed all the same and the module delivers
    // its copy. Exactly one decision is applied.
    let rig = Rig::new(ScriptedTriage::new(Vec::<Step>::new())).await;
    let (ambiguous, _bus) = ambiguous_client().await;
    let (offered, claim) =
        offer_or_direct_with(&ambiguous, event("u5"), &rig.claims, &ATTEMPT_DEADLINES).await;
    assert!(matches!(offered, Offered::Direct(ref why) if why.contains("ingest failed")));
    run_directly(&rig.triage, &event("u5")).await;
    claim.expect("the direct path is taken").finish(true);
    // The write that landed: the module holds the event and delivers it.
    let landed = event_queues::ingest(&rig.client, vec![event("u5")])
        .await
        .unwrap();
    assert_eq!(landed.results[0].disposition, Disposition::Accepted);
    let applied = rig.triage.until_applied(2, Duration::from_secs(2)).await;
    assert_eq!(applied.len(), 1, "triaged twice: {applied:?}");
    assert_eq!(rig.triage.runs(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_event_delivered_while_its_offer_waited_is_not_run_directly() {
    // The other order: the write lands and the module delivers it before
    // the offer gives up, so the caller must not run it directly.
    let rig = Rig::new(ScriptedTriage::new(Vec::<Step>::new())).await;
    let (ambiguous, _bus) = ambiguous_client().await;
    let offer = offer_or_direct_with(&ambiguous, event("u6"), &rig.claims, &ATTEMPT_DEADLINES);
    let (offered, landed) = tokio::join!(offer, async {
        let landed = event_queues::ingest(&rig.client, vec![event("u6")]).await;
        rig.triage.until_applied(1, Duration::from_secs(5)).await;
        landed
    });
    assert_eq!(
        landed.unwrap().results[0].disposition,
        Disposition::Accepted
    );
    let (offered, claim) = offered;
    assert_eq!(offered, Offered::Queued(Disposition::Duplicate));
    assert!(claim.is_none());
    assert_eq!(rig.triage.applied().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ingest_that_landed_still_runs_once_when_the_direct_path_fails() {
    // The direct triage fails, so the queues' copy is not dropped as
    // handled: it is delivered and decided, once.
    let rig = Rig::new(ScriptedTriage::new(Vec::<Step>::new())).await;
    let (ambiguous, _bus) = ambiguous_client().await;
    let (offered, claim) =
        offer_or_direct_with(&ambiguous, event("u11"), &rig.claims, &ATTEMPT_DEADLINES).await;
    assert!(matches!(offered, Offered::Direct(_)));
    claim.expect("the direct path is taken").finish(false);
    let landed = event_queues::ingest(&rig.client, vec![event("u11")])
        .await
        .unwrap();
    assert_eq!(landed.results[0].disposition, Disposition::Accepted);
    let applied = rig.triage.until_applied(1, Duration::from_secs(5)).await;
    assert_eq!(applied.len(), 1, "the queued copy was lost");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_ambiguous_offer_of_an_event_does_not_run_it_directly_again() {
    // A platform redelivery during the same outage.
    let rig = Rig::new(ScriptedTriage::new(Vec::<Step>::new())).await;
    let (ambiguous, _bus) = ambiguous_client().await;
    let (_, first) =
        offer_or_direct_with(&ambiguous, event("u12"), &rig.claims, &ATTEMPT_DEADLINES).await;
    first
        .expect("the first offer takes the direct path")
        .finish(true);
    let (again, second) =
        offer_or_direct_with(&ambiguous, event("u12"), &rig.claims, &ATTEMPT_DEADLINES).await;
    assert_eq!(again, Offered::Queued(Disposition::Duplicate));
    assert!(second.is_none());
}

#[tokio::test]
async fn an_ingest_that_surely_failed_still_takes_the_direct_path() {
    // No new loss: the module never saw the event, so no claim is taken
    // and a later offer of the same event is delivered by the queues.
    let workspace = tempfile::tempdir().unwrap();
    let config = config_in(&workspace);
    let mut off = config.clone();
    off.modules.enabled = false;
    let client = crate::modules::ops::client(off);
    let (offered, claim) = offer_or_direct(&config, &client, event("u7")).await;
    assert!(matches!(offered, Offered::Direct(ref why) if why.contains("ingest failed")));
    assert!(claim.is_none());
    let claims = Claims::of_workspace(&config.workspace_dir, 0);
    assert_eq!(claims.delivering(&event("u7").dedupe_key()), Delivery::Run);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_offer_without_a_direct_path_leaves_its_event_to_the_queues() {
    // `inbound.offer` reports the failure and the pusher offers again, so
    // an ambiguous offer must not mark its event as handled.
    let workspace = tempfile::tempdir().unwrap();
    let config = config_in(&workspace);
    let (ambiguous, _bus) = ambiguous_client().await;
    let offered = offer(&config, &ambiguous, event("u8")).await;
    assert!(!offered.is_queued());
    let claims = Claims::of_workspace(&config.workspace_dir, 0);
    assert_eq!(claims.delivering(&event("u8").dedupe_key()), Delivery::Run);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ambiguous_offer_claims_its_event_in_its_own_workspace() {
    // The direct-path claim lands in the offering config's workspace, where
    // that workspace's `Deliver` looks, and in no other.
    let (mine, other) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let config = config_in(&mine);
    let (ambiguous, _bus) = ambiguous_client().await;
    let (offered, claim) = offer_or_direct(&config, &ambiguous, event("u9")).await;
    assert!(matches!(offered, Offered::Direct(ref why) if why.contains("ingest failed")));
    let key = event("u9").dedupe_key();
    let theirs = Claims::of_workspace(&config_in(&other).workspace_dir, 0);
    let mine = Claims::of_workspace(&config.workspace_dir, 0);
    assert_eq!(mine.delivering(&key), Delivery::Wait);
    assert_eq!(theirs.delivering(&key), Delivery::Run);
    claim.expect("the direct path is taken").finish(true);
    assert_eq!(mine.delivering(&key), Delivery::Skip);
}

#[tokio::test]
async fn inactive_queues_send_a_direct_path_offer_down_it_without_a_claim() {
    let workspace = tempfile::tempdir().unwrap();
    let mut config = config_in(&workspace);
    config.inbound.enabled = false;
    let client = crate::modules::ops::client(config.clone());
    let (offered, claim) = offer_or_direct(&config, &client, event("u10")).await;
    assert!(matches!(offered, Offered::Direct(ref why) if why.contains("inactive")));
    assert!(claim.is_none());
    let claims = Claims::of_workspace(&config.workspace_dir, 0);
    assert_eq!(claims.delivering(&event("u10").dedupe_key()), Delivery::Run);
}

/// The active config, over its own workspace.
fn config_in(workspace: &tempfile::TempDir) -> crate::config::Config {
    let mut config = active_config();
    config.workspace_dir = workspace.path().to_path_buf();
    config
}

#[test]
fn inbound_events_belong_to_the_inbound_domain_and_name_themselves() {
    use crate::core::events::DomainEvent;
    for (event, name) in [
        (
            DomainEvent::InboundEventDropped {
                source: "composio:gmail".into(),
                event_id: "e1".into(),
                reason: "duplicate".into(),
            },
            "InboundEventDropped",
        ),
        (
            DomainEvent::InboundDeferred {
                source: "composio:gmail".into(),
                event_id: "e1".into(),
                defer_until_ms: 1_000,
                reason: "busy".into(),
            },
            "InboundDeferred",
        ),
        (
            DomainEvent::InboundDeadLettered {
                source: "webhook:t1".into(),
                event_id: "c1".into(),
                attempt: 5,
                detail: "terminal".into(),
            },
            "InboundDeadLettered",
        ),
    ] {
        assert_eq!(event.domain(), "inbound");
        assert_eq!(event.variant_name(), name);
    }
}

#[tokio::test]
async fn a_module_that_never_saw_the_event_is_not_sent_it_again() {
    // Modules off for the client: every call is refused as unavailable, which
    // proves the event never reached the queues, so there is nothing to
    // re-send and the direct path runs at once.
    let mut off = active_config();
    off.modules.enabled = false;
    let client = crate::modules::ops::client(off);
    let started = std::time::Instant::now();
    let offered = offer(&active_config(), &client, event("u4")).await;
    assert!(matches!(offered, Offered::Direct(ref why) if why.contains("ingest failed")));
    assert!(
        started.elapsed() < RESEND_PAUSE,
        "re-sent an event the module never saw: {:?}",
        started.elapsed()
    );
}
