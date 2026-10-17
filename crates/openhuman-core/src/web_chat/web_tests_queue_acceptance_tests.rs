use super::*;
use crate::core::runtime::DispatchCtx;

use serde_json::json;

async fn start_parked_turn(thread_id: &str, block: &TestRunChatTaskBlock) -> String {
    let request_id = start_chat(
        &crate::core::runtime::DispatchCtx::ambient(),
        "queue-test-client",
        thread_id,
        "primary turn",
        None,
        None,
        None,
        None,
        ChatRequestMetadata::default(),
    )
    .await
    .expect("primary turn should start");
    wait_for_flag(&block.started, "primary turn started").await;
    wait_for_in_flight(|entries| entries.iter().any(|(key, _)| key == thread_id)).await;
    request_id
}

async fn queue_message(thread_id: &str, text: &str, mode: &str) -> serde_json::Value {
    let response = start_chat(
        &crate::core::runtime::DispatchCtx::ambient(),
        "queue-test-client",
        thread_id,
        text,
        Some("model-override".to_string()),
        Some(0.7),
        Some("ar".to_string()),
        Some(mode.to_string()),
        ChatRequestMetadata::default(),
    )
    .await
    .expect("queued message should be accepted");
    serde_json::from_str(&response).expect("queued result should be JSON")
}

async fn cancel_parked_turn(thread_id: &str, block: &TestRunChatTaskBlock) {
    cancel_chat_scoped(
        &DispatchCtx::ambient(),
        "queue-test-client",
        thread_id,
        None,
    )
    .await
    .expect("parked turn should cancel");
    wait_for_in_flight(|entries| !entries.iter().any(|(key, _)| key == thread_id)).await;
    wait_for_flag(&block.dropped, "parked turn dropped during cleanup").await;
    set_test_run_chat_task_block(None).await;
}

#[tokio::test]
async fn web_queue_retains_typed_queued_turn_metadata() {
    let _serial = FORCED_ERROR_TEST_LOCK.lock().await;
    let block = make_block();
    set_test_run_chat_task_block(Some(block.clone())).await;
    let thread_id = "queue-typed-retention";
    start_parked_turn(thread_id, &block).await;

    let accepted = queue_message(thread_id, "defer this exact payload", "followup").await;
    assert_eq!(accepted["queued"], true);
    assert_eq!(accepted["queue_mode"], "followup");

    // This drains the actual in-flight host queue, not a standalone
    // RunQueue<String>, proving the concrete public DTO survives admission.
    let queued = drain_queued_turns_for_test(
        &crate::core::runtime::DispatchCtx::ambient(),
        thread_id,
        QueueLane::Followup,
    )
    .await;
    assert_eq!(queued.len(), 1);
    let queued = &queued[0];
    assert_eq!(queued.text, "defer this exact payload");
    assert_eq!(queued.client_id, "queue-test-client");
    assert_eq!(queued.thread_id, thread_id);
    assert!(queued.queued_at_ms > 0);
    assert_eq!(queued.model_override.as_deref(), Some("model-override"));
    assert_eq!(queued.temperature, Some(0.7));
    assert_eq!(queued.locale.as_deref(), Some("ar"));

    cancel_parked_turn(thread_id, &block).await;
}

#[tokio::test]
async fn interrupt_and_parallel_bypass_the_active_turn_queue() {
    let _serial = FORCED_ERROR_TEST_LOCK.lock().await;
    let block = make_block();
    set_test_run_chat_task_block(Some(block.clone())).await;
    let thread_id = "queue-disposition-bypass";
    let initial_request_id = start_parked_turn(thread_id, &block).await;

    let replacement_request_id = start_chat(
        &crate::core::runtime::DispatchCtx::ambient(),
        "queue-test-client",
        thread_id,
        "replace the active turn",
        None,
        None,
        None,
        Some("interrupt".to_string()),
        ChatRequestMetadata::default(),
    )
    .await
    .expect("interrupt should start a replacement turn");
    assert_ne!(replacement_request_id, initial_request_id);
    wait_for_in_flight(|entries| {
        entries
            .iter()
            .any(|(key, request_id)| key == thread_id && request_id == &replacement_request_id)
    })
    .await;

    start_chat(
        &crate::core::runtime::DispatchCtx::ambient(),
        "queue-test-client",
        thread_id,
        "run alongside the replacement",
        None,
        None,
        None,
        Some("parallel".to_string()),
        ChatRequestMetadata::default(),
    )
    .await
    .expect("parallel should start a forked turn");
    wait_for_parallel(|entries| entries.iter().any(|(_, key)| key == thread_id)).await;

    let status = channel_web_queue_status(&DispatchCtx::ambient(), thread_id)
        .await
        .expect("queue status should be available");
    assert_eq!(status.value["request_id"], replacement_request_id);
    assert_eq!(status.value["total"], 0);
    assert_eq!(status.value["steers"], 0);
    assert_eq!(status.value["followups"], 0);
    assert_eq!(status.value["collects"], 0);

    cancel_parked_turn(thread_id, &block).await;
    wait_for_parallel(|entries| !entries.iter().any(|(_, key)| key == thread_id)).await;
}

#[tokio::test]
async fn terminal_turn_dispatches_followup_as_a_fresh_host_turn() {
    let _serial = FORCED_ERROR_TEST_LOCK.lock().await;
    let block = make_block();
    set_test_run_chat_task_block(Some(block.clone())).await;
    let thread_id = "queue-followup-terminal";
    let primary_request_id = start_parked_turn(thread_id, &block).await;
    let accepted = queue_message(thread_id, "start after terminal", "followup").await;
    assert_eq!(accepted["queued"], true);

    // End the primary normally from the host test seam. The queue's follow-up
    // lane is drained only after terminal handling, then `dispatch_followups`
    // must route it back through `start_chat` as a fresh in-flight turn.
    block.release.notify_one();
    let entries = wait_for_in_flight(|entries| {
        entries
            .iter()
            .any(|(key, request_id)| key == thread_id && request_id != &primary_request_id)
    })
    .await;
    assert!(entries
        .iter()
        .any(|(key, request_id)| key == thread_id && request_id != &primary_request_id));

    cancel_parked_turn(thread_id, &block).await;
}

#[tokio::test]
async fn cancellation_discards_pending_lanes_before_a_replacement_turn() {
    let _serial = FORCED_ERROR_TEST_LOCK.lock().await;
    let cancelled_block = make_block();
    set_test_run_chat_task_block(Some(cancelled_block.clone())).await;
    let thread_id = "queue-cancelled-lanes";
    let cancelled_request_id = start_parked_turn(thread_id, &cancelled_block).await;

    // Queue every lane against the active turn. In particular, do not clear
    // these lanes: this test verifies that the real cancellation cleanup owns
    // their disposal.
    for (text, mode) in [
        ("followup must not survive", "followup"),
        ("steer must not survive", "steer"),
        ("collect must not survive", "collect"),
    ] {
        assert_eq!(queue_message(thread_id, text, mode).await["queued"], true);
    }
    assert_eq!(
        channel_web_queue_status(&DispatchCtx::ambient(), thread_id)
            .await
            .expect("queued status should be available")
            .value["total"],
        3
    );

    // Exercise the RPC-facing cancellation path used by the host, rather than
    // reaching into the queue or clearing it first.
    let cancelled = channel_web_cancel(
        &DispatchCtx::ambient(),
        "queue-test-client",
        thread_id,
        Some(&cancelled_request_id),
    )
    .await
    .expect("host cancellation should succeed");
    assert_eq!(cancelled.value["cancelled"], true);
    assert_eq!(cancelled.value["request_id"], cancelled_request_id);
    wait_for_in_flight(|entries| !entries.iter().any(|(key, _)| key == thread_id)).await;
    wait_for_flag(
        &cancelled_block.dropped,
        "cancelled turn should release its queue-owning task",
    )
    .await;

    // A newly-started host turn owns a newly-created queue. No cancelled
    // follow-up, steer, or collect payload may reappear in it or be delivered
    // to it later.
    set_test_run_chat_task_block(None).await;
    let replacement_block = make_block();
    set_test_run_chat_task_block(Some(replacement_block.clone())).await;
    let replacement_request_id = start_parked_turn(thread_id, &replacement_block).await;
    assert_ne!(replacement_request_id, cancelled_request_id);
    let replacement = channel_web_queue_status(&DispatchCtx::ambient(), thread_id)
        .await
        .expect("replacement queue status should be available");
    assert_eq!(replacement.value["total"], 0);
    assert!(drain_queued_turns_for_test(
        &crate::core::runtime::DispatchCtx::ambient(),
        thread_id,
        QueueLane::Followup
    )
    .await
    .is_empty());
    assert!(drain_queued_turns_for_test(
        &crate::core::runtime::DispatchCtx::ambient(),
        thread_id,
        QueueLane::Steer
    )
    .await
    .is_empty());
    assert!(drain_queued_turns_for_test(
        &crate::core::runtime::DispatchCtx::ambient(),
        thread_id,
        QueueLane::Collect
    )
    .await
    .is_empty());

    cancel_parked_turn(thread_id, &replacement_block).await;
}

#[tokio::test]
async fn web_queue_status_wire_shape_and_clear_cleanup_remain_stable() {
    let _serial = FORCED_ERROR_TEST_LOCK.lock().await;
    let block = make_block();
    set_test_run_chat_task_block(Some(block.clone())).await;
    let thread_id = "queue-wire-and-clear";
    let request_id = start_parked_turn(thread_id, &block).await;

    for (text, mode) in [
        ("steer payload", "steer"),
        ("followup payload", "followup"),
        ("collect payload", "collect"),
    ] {
        assert_eq!(queue_message(thread_id, text, mode).await["queued"], true);
    }

    let active = channel_web_queue_status(&DispatchCtx::ambient(), thread_id)
        .await
        .expect("active queue status should be available");
    let active_json = active
        .into_cli_compatible_json()
        .expect("queue status should serialize");
    // `items` carries a per-item minted uuid, so it can't be pinned by exact
    // whole-payload equality the way the scalar counts below are — check the
    // scalar shape exactly, then the items separately.
    let mut active_result = active_json["result"].clone();
    let items = active_result
        .as_object_mut()
        .expect("result object")
        .remove("items")
        .expect("items field present");
    assert_eq!(
        json!({ "result": active_result, "logs": active_json["logs"].clone() }),
        json!({
            "result": {
                "thread_id": thread_id,
                "active": true,
                "request_id": request_id,
                "steers": 1,
                "followups": 1,
                "collects": 1,
                "total": 3,
            },
            "logs": ["queue status retrieved"],
        })
    );
    let items = items.as_array().expect("items array");
    assert_eq!(items.len(), 3, "{items:?}");
    let mut lanes: Vec<&str> = items
        .iter()
        .map(|item| item["lane"].as_str().expect("lane"))
        .collect();
    lanes.sort_unstable();
    assert_eq!(lanes, ["collect", "followup", "steer"]);
    for item in items {
        assert!(!item["id"].as_str().expect("id").is_empty());
        assert!(item["text_preview"]
            .as_str()
            .expect("text_preview")
            .ends_with("payload"));
    }

    let cleared = channel_web_queue_clear(&DispatchCtx::ambient(), thread_id)
        .await
        .expect("queue clear should be available");
    assert_eq!(
        cleared
            .into_cli_compatible_json()
            .expect("queue clear should serialize"),
        json!({
            "result": {"thread_id": thread_id, "cleared": true, "dropped": 3},
            "logs": ["queue cleared"],
        })
    );
    assert_eq!(
        channel_web_queue_status(&DispatchCtx::ambient(), thread_id)
            .await
            .expect("empty queue status should be available")
            .into_cli_compatible_json()
            .expect("empty queue status should serialize"),
        json!({
            "result": {
                "thread_id": thread_id,
                "active": true,
                "request_id": request_id,
                "steers": 0,
                "followups": 0,
                "collects": 0,
                "total": 0,
                "items": [],
            },
            "logs": ["queue status retrieved"],
        })
    );

    cancel_parked_turn(thread_id, &block).await;
    assert_eq!(
        channel_web_queue_status(&DispatchCtx::ambient(), thread_id)
            .await
            .expect("inactive queue status should be available")
            .into_cli_compatible_json()
            .expect("inactive queue status should serialize"),
        json!({
            "result": {
                "thread_id": thread_id,
                "active": false,
                "steers": 0,
                "followups": 0,
                "collects": 0,
                "total": 0,
                "items": [],
            },
            "logs": ["no active turn for thread"],
        })
    );
}

/// `channel.web_queue_remove` retracts exactly the named item, leaving the
/// rest of the queue untouched, and emits `queue_item_removed`.
#[tokio::test]
async fn web_queue_remove_retracts_one_item_and_emits_event() {
    let _serial = FORCED_ERROR_TEST_LOCK.lock().await;
    let block = make_block();
    set_test_run_chat_task_block(Some(block.clone())).await;
    let thread_id = "queue-remove-one-item";
    start_parked_turn(thread_id, &block).await;

    assert_eq!(
        queue_message(thread_id, "keep me", "followup").await["queued"],
        true
    );
    assert_eq!(
        queue_message(thread_id, "remove me", "steer").await["queued"],
        true
    );

    let status = channel_web_queue_status(&DispatchCtx::ambient(), thread_id)
        .await
        .expect("status")
        .into_cli_compatible_json()
        .expect("status json");
    let items = status["result"]["items"].as_array().expect("items");
    assert_eq!(items.len(), 2);
    let target_id = items
        .iter()
        .find(|item| item["lane"] == "steer")
        .expect("steer item")["id"]
        .as_str()
        .expect("id")
        .to_string();

    let removed = channel_web_queue_remove(
        &DispatchCtx::ambient(),
        "queue-test-client",
        thread_id,
        &target_id,
    )
    .await
    .expect("remove")
    .into_cli_compatible_json()
    .expect("remove json");
    assert_eq!(
        removed,
        json!({
            "result": {
                "thread_id": thread_id,
                "item_id": target_id,
                "removed": true,
            },
            "logs": ["queue item remove processed"],
        })
    );

    let status_after = channel_web_queue_status(&DispatchCtx::ambient(), thread_id)
        .await
        .expect("status after remove")
        .into_cli_compatible_json()
        .expect("status after remove json");
    let items_after = status_after["result"]["items"].as_array().expect("items");
    assert_eq!(items_after.len(), 1);
    assert_eq!(items_after[0]["lane"], "followup");

    // Removing an id that no longer exists is a no-op, not an error.
    let removed_again = channel_web_queue_remove(
        &DispatchCtx::ambient(),
        "queue-test-client",
        thread_id,
        &target_id,
    )
    .await
    .expect("remove again")
    .into_cli_compatible_json()
    .expect("remove again json");
    assert_eq!(removed_again["result"]["removed"], false);

    cancel_parked_turn(thread_id, &block).await;
}

/// `chat_done` is the client's go-ahead for its next message, so it must not
/// go out while the finished turn still holds the thread's in-flight slot: a
/// message sent on it would interrupt the finished request and publish a
/// "cancelled" `chat_error` for it. The test holds the slot map's lock while
/// the turn completes; `chat_done` may only arrive after the lock is dropped,
/// and by then the slot is gone.
#[tokio::test]
async fn chat_done_is_published_only_after_the_in_flight_slot_is_released() {
    let _serial = FORCED_ERROR_TEST_LOCK.lock().await;
    let workspace = tempfile::tempdir().expect("workspace");
    let block = TestRunChatTaskBlock {
        succeed_in: Some(workspace.path().to_path_buf()),
        ..make_block()
    };
    set_test_run_chat_task_block(Some(block.clone())).await;
    let thread_id = "chat-done-after-slot-release";
    let request_id = start_parked_turn(thread_id, &block).await;
    let mut events = crate::web_chat::subscribe_web_channel_events();
    let is_done = |event: &crate::web_chat::WebChannelEvent| {
        event.event == "chat_done" && event.request_id == request_id
    };

    let held = crate::web_chat::ops::in_flight(&crate::core::runtime::DispatchCtx::ambient())
        .lock_owned()
        .await;
    block.release.notify_one();
    let early = timeout(Duration::from_millis(500), async {
        loop {
            if let Ok(event) = events.recv().await {
                if is_done(&event) {
                    return event;
                }
            }
        }
    })
    .await;
    assert!(
        early.is_err(),
        "chat_done went out while the turn still held its in-flight slot"
    );
    drop(held);

    timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(event) = events.recv().await {
                if is_done(&event) {
                    return;
                }
            }
        }
    })
    .await
    .expect("chat_done after the slot is released");
    assert!(
        !in_flight_entries_for_test(&crate::core::runtime::DispatchCtx::ambient())
            .await
            .iter()
            .any(|(key, _)| key == thread_id),
        "the slot is released by the time chat_done is seen"
    );
    set_test_run_chat_task_block(None).await;
}
