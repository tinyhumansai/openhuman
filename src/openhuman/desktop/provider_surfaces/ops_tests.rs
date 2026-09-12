use super::*;
use std::sync::Mutex;

/// Serializes tests that mutate the process-global RESPOND_QUEUE so cargo's
/// default parallel test runner cannot interleave clear/insert/assert cycles.
static TEST_MUTEX: Mutex<()> = Mutex::new(());

fn sample_event(entity_id: &str) -> ProviderEvent {
    ProviderEvent {
        provider: "linkedin".into(),
        account_id: "acct-1".into(),
        event_kind: "message".into(),
        entity_id: entity_id.into(),
        thread_id: Some("thread-1".into()),
        title: Some("New message".into()),
        snippet: Some("Can we talk tomorrow?".into()),
        sender_name: Some("Taylor".into()),
        sender_handle: Some("taylor".into()),
        timestamp: "2026-04-22T16:55:00Z".into(),
        deep_link: Some("https://www.linkedin.com/messaging/thread-1".into()),
        requires_attention: true,
        raw_payload: None,
    }
}

#[tokio::test]
async fn ingest_event_upserts_queue_item() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
    store::clear_queue();
    let first = ingest_event(sample_event("entity-1")).await.unwrap();
    let second = ingest_event(sample_event("entity-1")).await.unwrap();

    let first_value = first.into_cli_compatible_json().unwrap();
    let second_value = second.into_cli_compatible_json().unwrap();
    let first_result = first_value.get("data").unwrap_or(&first_value);
    let second_result = second_value.get("data").unwrap_or(&second_value);

    assert_eq!(first_result["provider"], "linkedin");
    assert_eq!(second_result["entity_id"], "entity-1");

    let queue = list_queue(EmptyRequest {}).await.unwrap();
    let queue_json = queue.into_cli_compatible_json().unwrap();
    let data = queue_json.get("data").unwrap_or(&queue_json);
    assert_eq!(data["count"], 1);
}

#[tokio::test]
async fn list_queue_returns_newest_first() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
    store::clear_queue();
    ingest_event(sample_event("entity-1")).await.unwrap();
    ingest_event(sample_event("entity-2")).await.unwrap();

    let queue = list_queue(EmptyRequest {}).await.unwrap();
    let queue_json = queue.into_cli_compatible_json().unwrap();
    let data = queue_json.get("data").unwrap_or(&queue_json);
    let items = data["items"].as_array().unwrap();

    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["entity_id"], "entity-2");
    assert_eq!(items[1]["entity_id"], "entity-1");
}
