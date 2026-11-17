//! Tests for the `tinyeventqueues` module adapter, against the linked module.

use std::time::Duration;

use serde_json::json;

use super::*;
use crate::inbound::events;
use crate::inbound::fixture::{Rig, ScriptedTriage, Step};
use crate::triage::TriggerEnvelope;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_member_is_reachable() {
    let rig = Rig::new(ScriptedTriage::new(Vec::<Step>::new())).await;
    let envelope = TriggerEnvelope::from_composio("notion", "PAGE", "", "u", json!({}));
    let event = events::composio(&envelope).unwrap();

    let ingested = ingest(&rig.client, vec![event]).await.unwrap();
    assert_eq!(ingested.results.len(), 1);
    let _ = drain(&rig.client).await.unwrap();
    let _ = rig.triage.until_applied(1, Duration::from_secs(5)).await;
    let depths = stats(&rig.client).await.unwrap();
    assert_eq!(depths.queues.len(), 6);
    assert!(list_dead(&rig.client, 5).await.unwrap().is_empty());
    assert!(!redrive(&rig.client, "teq.dead/none").await.unwrap());
}
