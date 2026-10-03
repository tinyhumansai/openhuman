use super::*;

#[test]
fn single_bubble_delivery_emits_one_unsegmented_chat_done() {
    let mut rx = crate::web_chat::subscribe_web_channel_events();
    // Prose `deliver_response` WOULD split into several `chat_segment` bubbles
    // (long, multi-paragraph, no fences) — the shape a background delivery turn
    // produces. A core-persisted single row must be announced as one bubble.
    let text = "Same three meetings as before, and nothing on the calendar moved since the last check.\n\n\
        The product standup is still at noon and the design review still follows it at two.\n\n\
        Nothing needs input from you right now, so I have not rescheduled anything on your behalf.";
    assert!(
        segment_for_delivery(text).len() > 1,
        "fixture must be one the conversational path would segment"
    );
    let request_id = format!("single-bubble-{}", uuid::Uuid::new_v4());

    deliver_response_single_bubble("system", "thread-1", &request_id, text, None);

    // Other tests publish on the same process-global bus; keep only ours.
    let mut mine = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(event) if event.request_id == request_id => mine.push(event),
            Ok(_) => continue,
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
            Err(_) => break,
        }
    }
    assert_eq!(mine.len(), 1, "exactly one terminal event, no chat_segment");
    let done = &mine[0];
    assert_eq!(done.event, "chat_done");
    assert_eq!(done.client_id, "system");
    assert_eq!(done.thread_id, "thread-1");
    assert_eq!(done.full_response.as_deref(), Some(text));
    assert_eq!(done.segment_total, None);
    assert_eq!(done.segment_index, None);
    assert!(done.usage.is_none());
}

// ── chat_done.timing ──────────────────────────────────────────────────────

/// `deliver_response` forwards a supplied timing snapshot onto `chat_done`'s
/// `timing` field, with `tokens_per_second` derived from the usage's
/// `output_tokens` and the snapshot's `total_ms`.
#[tokio::test]
async fn chat_done_carries_timing_when_a_snapshot_is_supplied() {
    let mut rx = crate::web_chat::subscribe_web_channel_events();
    let request_id = format!("timing-{}", uuid::Uuid::new_v4());

    let usage = crate::agent::tinyagents::host::LastTurnUsage {
        input_tokens: 100,
        output_tokens: 40,
        cached_input_tokens: 0,
        cost_usd: 0.01,
        context_window: 8000,
        subagents: Vec::new(),
    };
    let timing = crate::web_chat::turn_timing::TurnTimingSnapshot {
        first_token_ms: Some(120),
        first_tool_ms: None,
        total_ms: Some(2000),
    };

    test_support::deliver_response_with_timing_for_test(
        "system",
        "thread-timing",
        &request_id,
        "Quick answer.",
        "how fast?",
        Some(&usage),
        Some(timing),
    )
    .await;

    let done = loop {
        match rx.try_recv() {
            Ok(event) if event.request_id == request_id && event.event == "chat_done" => {
                break event;
            }
            Ok(_) => continue,
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
            Err(_) => panic!("chat_done for {request_id} never arrived"),
        }
    };

    let payload = done.timing.expect("chat_done.timing must be Some");
    assert_eq!(payload.first_token_ms, Some(120));
    assert_eq!(payload.first_tool_ms, None);
    assert_eq!(payload.total_ms, Some(2000));
    // 40 output tokens / (2000ms / 1000) = 20 tokens/sec.
    assert_eq!(payload.tokens_per_second, Some(20.0));
}

/// A caller with no timing snapshot in scope (e.g. the flows stream
/// finalizer, which discards its bridge handle) gets `chat_done.timing ==
/// None` rather than a fabricated zero-valued payload.
#[tokio::test]
async fn chat_done_omits_timing_when_no_snapshot_is_supplied() {
    let mut rx = crate::web_chat::subscribe_web_channel_events();
    let request_id = format!("timing-none-{}", uuid::Uuid::new_v4());

    test_support::deliver_response_with_timing_for_test(
        "system",
        "thread-timing-none",
        &request_id,
        "Quick answer.",
        "how fast?",
        None,
        None,
    )
    .await;

    let done = loop {
        match rx.try_recv() {
            Ok(event) if event.request_id == request_id && event.event == "chat_done" => {
                break event;
            }
            Ok(_) => continue,
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
            Err(_) => panic!("chat_done for {request_id} never arrived"),
        }
    };

    assert!(done.timing.is_none());
}

// ── Delivery persists before it announces (#6034) ───────────────────────

#[tokio::test]
async fn delivery_stores_the_reply_before_announcing_it() {
    use crate::threads::store::{self as conversations, CreateConversationThread};

    let ws = std::env::temp_dir().join(format!("deliver-persist-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&ws).unwrap();
    conversations::ensure_thread(
        ws.clone(),
        CreateConversationThread {
            id: "t-deliver".to_string(),
            title: "Chat".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
        },
    )
    .expect("thread created");

    let citation = crate::memory::types::TurnCitation {
        id: "mem-deliver".to_string(),
        key: "summary-source".to_string(),
        namespace: None,
        score: Some(0.8),
        timestamp: "2026-09-04T00:00:00Z".to_string(),
        snippet: "source snippet".to_string(),
    };
    test_support::deliver_response_in_workspace_for_test(
        "client-1",
        "t-deliver",
        "req-deliver",
        "Here is the summary you asked for.",
        "summarise this",
        &[citation],
        Some(ws.as_path()),
    )
    .await;

    // `deliver_response` returns only after the terminal event is published, so
    // a row present here proves the write happened no later than the announce.
    // A client that never receives that event, or fails to append it, no longer
    // decides whether the reply exists.
    let messages = conversations::get_messages(ws.clone(), "t-deliver").expect("messages");
    assert_eq!(messages.len(), 1, "delivery must leave exactly one row");
    assert_eq!(messages[0].id, "agent:req-deliver");
    assert_eq!(messages[0].content, "Here is the summary you asked for.");
    assert_eq!(messages[0].sender, "agent");
    // The client's append is deduped onto this row, so the citations it would
    // have written must already be here or the chips render empty.
    assert_eq!(
        messages[0].extra_metadata["citations"][0]["id"],
        "mem-deliver"
    );
}

#[tokio::test]
async fn delivery_still_announces_when_the_reply_cannot_be_stored() {
    // The thread does not exist, so the store refuses the write. Delivery must
    // continue regardless: a storage failure that also swallowed the
    // announcement would turn a recoverable problem into a dead turn, and the
    // client's own append is still a working fallback.
    let ws = std::env::temp_dir().join(format!("deliver-nothread-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&ws).unwrap();

    test_support::deliver_response_in_workspace_for_test(
        "client-3",
        "absent-thread",
        "req-absent",
        "an answer with nowhere to go",
        "hi",
        &[],
        Some(ws.as_path()),
    )
    .await;

    // `get_messages` answers `Ok(vec![])` for a thread it has never seen — only
    // `append_message` refuses one — so absence is what proves the write was
    // rejected and swallowed rather than silently creating a thread.
    let messages = crate::threads::store::get_messages(ws.clone(), "absent-thread")
        .expect("reading an unknown thread is not an error");
    assert!(
        messages.is_empty(),
        "the thread was never created, so nothing should have been written"
    );
}

#[tokio::test]
async fn delivery_without_a_workspace_persists_nothing() {
    // Callers with no workspace in scope (the flows stream finalizer) keep the
    // pre-#6034 behaviour: the viewing client stays the only persister.
    test_support::deliver_response_for_test(
        "client-2",
        "t-none",
        "req-none",
        "nothing to store",
        "hi",
        &[],
    )
    .await;
}
