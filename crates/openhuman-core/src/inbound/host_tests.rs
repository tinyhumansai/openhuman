//! Tests for the served host callbacks.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tinybus::Interface;
use tinyeventqueues_bus::{DeliverStatus, Lane, Turn};

use super::*;
use crate::inbound::events;
use crate::inbound::fixture::{decision, ScriptedTriage, Step};
use crate::triage::{TriageAction, TriageRunner, TriggerEnvelope};

fn host(triage: &Arc<ScriptedTriage>) -> InboundHost {
    InboundHost::new(Arc::new(Deliverer::new(
        triage.clone() as Arc<dyn TriageRunner>,
        DispatchCtx::host_default(),
        Duration::from_millis(200),
    )))
}

fn trigger() -> tinyeventqueues_bus::InboundEvent {
    let envelope = TriggerEnvelope::from_composio("jira", "JIRA_ISSUE", "", "u", json!({}));
    events::composio(&envelope).unwrap()
}

#[test]
fn the_interface_is_the_contract() {
    let host = host(&ScriptedTriage::new(Vec::<Step>::new()));
    assert_eq!(host.name().to_string(), NAME);
    let members: Vec<String> = host.members().into_iter().map(|m| m.to_string()).collect();
    assert_eq!(members, tinyeventqueues_bus::names::host::METHODS.to_vec());
}

#[tokio::test]
async fn deliver_and_classify_go_to_the_deliverer() {
    let triage = ScriptedTriage::new([decision(TriageAction::React), Step::Fail("down")]);
    let host = host(&triage);
    let delivered = host
        .deliver(DeliverRequest::new(Turn::new(
            "t",
            Lane::Background,
            "c",
            vec![trigger()],
        )))
        .await
        .unwrap();
    assert_eq!(delivered.status, DeliverStatus::Accepted);
    let refused = host
        .classify_salience(ClassifySalienceRequest::new(trigger(), 35))
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("triage failed"), "{refused}");
}

#[tokio::test]
async fn install_serves_the_callbacks_under_the_contract_name() {
    let bus = tinybus::transport::memory::MemoryBus::new();
    tinybus::broker::Broker::new().spawn(bus.clone());
    let connection = tinybus::Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    install(&connection).await.unwrap();
    let names = connection.list_names().await.unwrap();
    assert!(names.iter().any(|name| name.as_str() == NAME));
}
