//! Tests for running a pinned source's batch: the prompt, and resuming the
//! runs a previous process left unfinished.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use super::*;
use crate::inbound::fixture::{memory_results, until_finished, Reply, ScriptedAgent};
use crate::inbound::results::{ResultStatus, Stored};

fn item(id: &str, text: &str) -> ExternalItem {
    ExternalItem {
        id: id.to_owned(),
        text: text.to_owned(),
        payload: json!({ "author": "@a" }),
        received_at_ms: 7,
    }
}

#[test]
fn the_default_prompt_is_an_instruction_and_the_batch_as_json() {
    let source = InboundSourceConfig::default();
    let prompt = render_prompt("x:mentions", "mentions", &source, &[item("1", "hi")]);
    assert!(prompt.contains("x:mentions"), "{prompt}");
    let json_start = prompt.find('[').unwrap();
    let events: serde_json::Value = serde_json::from_str(&prompt[json_start..]).unwrap();
    assert_eq!(
        events,
        json!([{ "id": "1", "text": "hi", "payload": { "author": "@a" }, "received_at_ms": 7 }])
    );
}

#[test]
fn a_configured_prompt_takes_the_batch_in_place_of_its_placeholder() {
    let source = InboundSourceConfig {
        prompt: Some("Draft replies.\n{{events}}\nBe brief.".to_owned()),
        ..InboundSourceConfig::default()
    };
    let prompt = render_prompt(
        "x:mentions",
        "mentions",
        &source,
        &[item("1", "a"), item("2", "b")],
    );
    assert!(prompt.starts_with("Draft replies.\n[{"), "{prompt}");
    assert!(prompt.ends_with("}]\nBe brief."), "{prompt}");
    assert!(!prompt.contains("{{events}}"));
}

#[tokio::test]
async fn runs_left_running_by_a_previous_process_are_run_again() {
    let results = memory_results();
    results
        .start(&Stored::running(
            "t-left",
            "x:mentions",
            "mentions",
            "teeny-mentions",
            vec!["1".into()],
            "the stored message",
            5,
        ))
        .unwrap();
    let agent = ScriptedAgent::new([Reply::Text("resumed")]);
    let pinned = Pinned::new(agent.clone() as Arc<dyn PinnedAgent>, results.clone());
    assert_eq!(pinned.resume(&DispatchCtx::host_default()), 1);
    let done = until_finished(&results, "t-left", Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(done.result.status, ResultStatus::Completed);
    assert_eq!(done.result.text.as_deref(), Some("resumed"));
    let runs = agent.runs();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].message, "the stored message");
    assert_eq!(runs[0].agent_id, "teeny-mentions");
    // Nothing is left to resume.
    assert_eq!(pinned.resume(&DispatchCtx::host_default()), 0);
}

#[test]
fn the_turn_events_belong_to_the_inbound_domain_and_say_what_ran() {
    use crate::core::events::DomainEvent;
    let completed = DomainEvent::InboundTurnCompleted {
        source: "x:mentions".into(),
        key: "mentions".into(),
        turn_id: "t1".into(),
        event_ids: vec!["1".into(), "2".into()],
        agent_id: "teeny-mentions".into(),
    };
    assert_eq!(completed.domain(), "inbound");
    assert_eq!(completed.variant_name(), "InboundTurnCompleted");
    assert_eq!(
        completed.log_detail().as_deref(),
        Some("x:mentions/mentions turn t1: 2 event(s)")
    );
    let failed = DomainEvent::InboundTurnFailed {
        source: "x:mentions".into(),
        key: "mentions".into(),
        turn_id: "t1".into(),
        detail: "provider down".into(),
    };
    assert_eq!(failed.domain(), "inbound");
    assert_eq!(failed.variant_name(), "InboundTurnFailed");
    assert_eq!(
        failed.log_detail().as_deref(),
        Some("x:mentions/mentions turn t1: provider down")
    );
}

#[tokio::test]
async fn a_failed_turn_stores_its_detail_scrubbed() {
    let results = memory_results();
    results
        .start(&Stored::running(
            "t-fail",
            "x:mentions",
            "mentions",
            "teeny-mentions",
            vec!["1".into()],
            "msg",
            5,
        ))
        .unwrap();
    let agent = ScriptedAgent::new([]);
    let pinned = Pinned::new(agent as Arc<dyn PinnedAgent>, results.clone());
    pinned.fail(
        "t-fail",
        "provider said 401 for Bearer sk-live-SECRETSECRETSECRET1234",
    );
    let stored = results.get("t-fail").unwrap().unwrap();
    let error = stored.result.error.unwrap();
    assert!(!error.contains("SECRETSECRET"), "{error}");
    assert!(error.contains("[REDACTED]"), "{error}");
}
