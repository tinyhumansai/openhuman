//! Tests for `inbound.stats`, `inbound.dead` and `inbound.redrive`.

use super::*;
use crate::inbound::fixture::{active_config, Rig, ScriptedTriage, Step};

#[tokio::test]
async fn the_component_switch_refuses_every_operation() {
    let mut config = active_config();
    config.components.eventqueues = false;
    let client = crate::modules::ops::client(config.clone());
    for refused in [
        stats(&config, &client).await.map(|_| ()),
        dead(&config, &client, None).await.map(|_| ()),
        redrive(&config, &client, "x").await.map(|_| ()),
    ] {
        assert!(refused
            .unwrap_err()
            .contains("COMPONENT_DISABLED:eventqueues"));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_operations_reach_the_module() {
    let rig = Rig::new(ScriptedTriage::new(Vec::<Step>::new())).await;
    let config = active_config();

    let depths = stats(&config, &rig.client).await.unwrap();
    assert!(depths.value.queues.iter().any(|q| q.queue == "teq.ingress"));

    let parked = dead(&config, &rig.client, Some(10_000)).await.unwrap();
    assert!(parked.value.items.is_empty());

    let gone = redrive(&config, &rig.client, "teq.dead/none")
        .await
        .unwrap();
    assert!(!gone.value.redriven);
    assert!(redrive(&config, &rig.client, " ")
        .await
        .unwrap_err()
        .contains("id is required"));
    assert!(redrive(&config, &rig.client, "nope")
        .await
        .unwrap_err()
        .starts_with("inbound.redrive:"));
}

// ── External sources: offer, results, ack ──────────────────────────────────

use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;
use tinybus::EventHandler;

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::inbound::fixture::{memory_results, mentions_inbound, Reply, ScriptedAgent};
use crate::inbound::pinned::{Pinned, PinnedAgent};
use crate::inbound::results::{ResultStatus, Results};

fn mentions_config(debounce_ms: u64, max_batch: u32) -> crate::config::Config {
    let mut config = active_config();
    config.inbound = mentions_inbound(debounce_ms, max_batch);
    config
}

/// Distinct texts, so the queues' habituation does not decay later mentions.
const TEXTS: [&str; 5] = [
    "could you summarise the release notes for version two",
    "my order arrived broken, what should I do next",
    "is the conference keynote being streamed anywhere tonight",
    "loved the podcast episode about gardening on balconies",
    "which hiking trail near the lake would you recommend",
];

fn mention(id: &str) -> OfferParams {
    let n: usize = id.parse().unwrap_or(0);
    serde_json::from_value(json!({
        "source": "x:mentions",
        "key": "mentions",
        "event_id": id,
        "text": format!("@bot {}", TEXTS[n % TEXTS.len()]),
        "payload": { "author": format!("@user{id}") },
    }))
    .unwrap()
}

/// Collects the `inbound` domain events of one source.
#[derive(Clone)]
struct Completed(Arc<StdMutex<Vec<DomainEvent>>>, &'static str);

#[async_trait]
impl EventHandler<DomainEvent> for Completed {
    fn name(&self) -> &str {
        "test::inbound_completed"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["inbound"])
    }

    async fn handle(&self, event: &DomainEvent) {
        if let DomainEvent::InboundTurnCompleted { turn_id, .. } = event {
            if turn_id.starts_with(self.1) || self.1.is_empty() {
                self.0.lock().unwrap().push(event.clone());
            }
        }
    }
}

async fn until_results(
    config: &crate::config::Config,
    results: &Results,
    n: usize,
) -> Vec<crate::inbound::results::InboundResult> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let listed = super::results(
            config,
            results,
            ResultsParams {
                status: Some(ResultStatus::Completed),
                ..ResultsParams::default()
            },
        )
        .unwrap()
        .value
        .results;
        if listed.len() >= n || tokio::time::Instant::now() >= deadline {
            return listed;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn an_offer_names_an_unconfigured_or_unusable_source() {
    let config = mentions_config(10, 25);
    let client = crate::modules::ops::client(config.clone());
    let mut params = mention("1");
    params.source = "x:dms".into();
    let refused = offer(&config, &client, params).await.unwrap_err();
    assert!(refused.contains("not configured"), "{refused}");
    let mut params = mention("1");
    params.source = "composio:gmail".into();
    assert!(offer(&config, &client, params)
        .await
        .unwrap_err()
        .contains("reserved"));
    let mut params = mention("1");
    params.event_id = " ".into();
    assert!(offer(&config, &client, params)
        .await
        .unwrap_err()
        .contains("event_id"));
    let mut off = config.clone();
    off.components.eventqueues = false;
    assert!(offer(&off, &client, mention("1"))
        .await
        .unwrap_err()
        .contains("COMPONENT_DISABLED:eventqueues"));
    let results = memory_results();
    assert!(super::results(&off, &results, ResultsParams::default()).is_err());
    assert!(result_ack(&off, &results, "t").is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_offers_within_the_window_run_one_turn_with_one_result_and_one_event() {
    crate::core::bus::init().await.expect("bus init");
    let seen = Completed(Arc::new(StdMutex::new(Vec::new())), "");
    let _handle = BUS.subscribe(Arc::new(seen.clone()));
    let config = mentions_config(300, 25);
    let agent = ScriptedAgent::new([Reply::Text("three drafts")]);
    let results = memory_results();
    let rig = Rig::pinned(
        config.inbound.clone(),
        Pinned::new(agent.clone() as Arc<dyn PinnedAgent>, results.clone()),
    )
    .await;

    for id in ["101", "102", "103"] {
        let offered = offer(&config, &rig.client, mention(id)).await.unwrap();
        assert_eq!(offered.value.disposition, "accepted");
    }
    // A redelivery of a mention is a success, not an error.
    let again = offer(&config, &rig.client, mention("102")).await.unwrap();
    assert_eq!(again.value.disposition, "duplicate");

    let listed = until_results(&config, &results, 1).await;
    assert_eq!(listed.len(), 1, "{listed:?}");
    let result = &listed[0];
    assert_eq!(result.source, "x:mentions");
    assert_eq!(result.key, "mentions");
    assert_eq!(result.agent_id, "teeny-mentions");
    assert_eq!(result.text.as_deref(), Some("three drafts"));
    assert_eq!(result.event_ids, ["101", "102", "103"]);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(agent.runs().len(), 1, "the batch ran more than once");
    let completed: Vec<_> = seen
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|event| {
            matches!(event, DomainEvent::InboundTurnCompleted { turn_id, .. } if *turn_id == result.turn_id)
        })
        .cloned()
        .collect();
    assert_eq!(completed.len(), 1);

    let acked = result_ack(&config, &results, &result.turn_id).unwrap();
    assert!(acked.value.acked);
    assert!(super::results(&config, &results, ResultsParams::default())
        .unwrap()
        .value
        .results
        .is_empty());
    assert!(result_ack(&config, &results, "unknown").is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_burst_over_max_batch_runs_in_turns_of_at_most_max_batch() {
    let config = mentions_config(300, 2);
    let agent = ScriptedAgent::new([]);
    let results = memory_results();
    let rig = Rig::pinned(
        config.inbound.clone(),
        Pinned::new(agent.clone() as Arc<dyn PinnedAgent>, results.clone()),
    )
    .await;
    for id in ["201", "202", "203", "204", "205"] {
        offer(&config, &rig.client, mention(id)).await.unwrap();
    }
    let listed = until_results(&config, &results, 3).await;
    let mut batches: Vec<Vec<String>> = listed.into_iter().map(|r| r.event_ids).collect();
    batches.sort();
    assert!(batches.iter().all(|batch| batch.len() <= 2), "{batches:?}");
    assert_eq!(batches.concat().len(), 5, "{batches:?}");
}

#[test]
fn results_are_limited_and_pruned_past_their_ttl() {
    let mut config = mentions_config(10, 25);
    config
        .inbound
        .sources
        .get_mut("x:mentions")
        .unwrap()
        .result_ttl_days = 1;
    let results = memory_results();
    let now = super::now_ms();
    let day = 24 * 60 * 60 * 1_000;
    for (turn, at) in [("old", now - 2 * day), ("t1", now - 10), ("t2", now - 5)] {
        results
            .start(&crate::inbound::results::Stored::running(
                turn,
                "x:mentions",
                "mentions",
                "teeny-mentions",
                vec![],
                "m",
                at,
            ))
            .unwrap();
    }
    let params: ResultsParams = serde_json::from_value(json!({ "limit": 1 })).unwrap();
    let listed = super::results(&config, &results, params)
        .unwrap()
        .value
        .results;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].turn_id, "t1");
    assert!(results.get("old").unwrap().is_none(), "not pruned");
    let capped: ResultsParams = serde_json::from_value(json!({ "limit": 100_000 })).unwrap();
    assert_eq!(
        super::results(&config, &results, capped)
            .unwrap()
            .value
            .results
            .len(),
        2
    );
}
