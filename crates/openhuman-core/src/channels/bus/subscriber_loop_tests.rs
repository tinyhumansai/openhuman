//! The reply loop projects shared-runner events onto a progressive reply sink.

use super::*;
use crate::web_chat::ScopedWebEvent;
use std::sync::Mutex;
use tokio::sync::broadcast;

#[derive(Default)]
struct Sink {
    batches: Mutex<Vec<Vec<ProgressiveEvent>>>,
    finalized: Mutex<Vec<String>>,
    fail_feed: bool,
}

#[async_trait::async_trait]
impl ProgressiveReplySink for Sink {
    async fn feed(&self, events: Vec<ProgressiveEvent>) -> Result<(), String> {
        if self.fail_feed {
            return Err("provider rejected the update".into());
        }
        self.batches.lock().unwrap().push(events);
        Ok(())
    }

    async fn finalize(&self, text: &str) -> Result<(), String> {
        self.finalized.lock().unwrap().push(text.to_owned());
        Ok(())
    }
}

impl Sink {
    fn events(&self) -> Vec<String> {
        self.batches
            .lock()
            .unwrap()
            .iter()
            .flatten()
            .map(|event| match event {
                ProgressiveEvent::TextDelta(t) => format!("text:{t}"),
                ProgressiveEvent::ThinkingDelta(t) => format!("thinking:{t}"),
                ProgressiveEvent::ToolCall(n) => format!("call:{n}"),
                ProgressiveEvent::ToolResult { tool_name, success } => {
                    format!("result:{tool_name}:{success}")
                }
            })
            .collect()
    }
    fn finals(&self) -> Vec<String> {
        self.finalized.lock().unwrap().clone()
    }
}

fn event(request: &str, kind: &str, fields: serde_json::Value) -> ScopedWebEvent {
    let mut value = serde_json::json!({
        "event": kind, "client_id": "c", "thread_id": "t", "request_id": request,
    });
    value
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    ScopedWebEvent::new(serde_json::from_value(value).expect("valid event"))
}

async fn run(events: Vec<ScopedWebEvent>, sink: &Sink, close_bus: bool) {
    let (tx, rx) = broadcast::channel(256);
    for e in events {
        tx.send(e).unwrap();
    }
    let session = tokio_util::sync::CancellationToken::new();
    let looped = run_channel_reply_loop(rx, "req-1".into(), "telegram".into(), session, sink);
    if close_bus {
        drop(tx);
        looped.await;
    } else {
        tokio::time::timeout(std::time::Duration::from_secs(5), looped)
            .await
            .expect("the loop ends at a terminal event");
        drop(tx);
    }
}

#[tokio::test]
async fn progress_is_batched_and_the_final_reply_finalizes_once() {
    let sink = Sink::default();
    run(
        vec![
            event(
                "req-1",
                "thinking_delta",
                serde_json::json!({"delta": "hmm"}),
            ),
            event("req-1", "text_delta", serde_json::json!({"delta": "Hel"})),
            event(
                "req-1",
                "tool_call",
                serde_json::json!({"tool_name": "search"}),
            ),
            event("req-1", "tool_call", serde_json::json!({})),
            event(
                "req-1",
                "tool_result",
                serde_json::json!({"tool_name": "search", "success": false}),
            ),
            event(
                "req-1",
                "tool_result",
                serde_json::json!({"tool_name": "fetch"}),
            ),
            event("req-1", "tool_result", serde_json::json!({})),
            event(
                "other-request",
                "text_delta",
                serde_json::json!({"delta": "NOT MINE"}),
            ),
            event("req-1", "iteration_start", serde_json::json!({})),
            event(
                "req-1",
                "chat_done",
                serde_json::json!({"full_response": "Hello!"}),
            ),
        ],
        &sink,
        false,
    )
    .await;

    assert_eq!(
        sink.events(),
        [
            "thinking:hmm",
            "text:Hel",
            "call:search",
            "result:search:false",
            "result:fetch:true",
        ]
    );
    assert_eq!(sink.finals(), ["Hello!"]);
}

#[tokio::test]
async fn an_empty_final_response_gets_a_visible_fallback() {
    let sink = Sink::default();
    run(
        vec![event(
            "req-1",
            "chat_done",
            serde_json::json!({"full_response": "  "}),
        )],
        &sink,
        false,
    )
    .await;
    assert_eq!(sink.finals(), ["(No response from agent.)"]);
    let sink = Sink::default();
    run(
        vec![event("req-1", "chat_done", serde_json::json!({}))],
        &sink,
        false,
    )
    .await;
    assert_eq!(sink.finals(), ["(No response from agent.)"]);
}

#[tokio::test]
async fn errors_and_cancellations_finalize_with_a_user_visible_message() {
    let sink = Sink::default();
    run(
        vec![
            event(
                "req-1",
                "text_delta",
                serde_json::json!({"delta": "partial"}),
            ),
            event(
                "req-1",
                "chat_error",
                serde_json::json!({"message": "model exploded"}),
            ),
        ],
        &sink,
        false,
    )
    .await;
    assert_eq!(
        sink.finals(),
        ["Sorry, I encountered an error: model exploded"]
    );
    assert_eq!(
        sink.events(),
        ["text:partial"],
        "pending progress flushes first"
    );

    let sink = Sink::default();
    run(
        vec![event("req-1", "chat_error", serde_json::json!({}))],
        &sink,
        false,
    )
    .await;
    assert_eq!(
        sink.finals(),
        ["Sorry, I encountered an error: unknown error"]
    );

    let sink = Sink::default();
    run(
        vec![
            event("req-1", "text_delta", serde_json::json!({"delta": "x"})),
            event(
                "req-1",
                "chat_cancelled",
                serde_json::json!({"cancel_reason": "user"}),
            ),
        ],
        &sink,
        false,
    )
    .await;
    assert_eq!(sink.finals(), ["Cancelled."]);
    assert_eq!(sink.events().len(), 1);
}

#[tokio::test]
async fn a_failed_progress_flush_ends_the_loop_without_a_final_message() {
    let sink = Sink {
        fail_feed: true,
        ..Default::default()
    };
    run(
        vec![
            event("req-1", "text_delta", serde_json::json!({"delta": "a"})),
            event(
                "req-1",
                "chat_done",
                serde_json::json!({"full_response": "done"}),
            ),
        ],
        &sink,
        false,
    )
    .await;
    assert!(
        sink.finals().is_empty(),
        "delivery failure stops the turn's projection"
    );

    for terminal in [
        event("req-1", "chat_error", serde_json::json!({"message": "m"})),
        event("req-1", "chat_cancelled", serde_json::json!({})),
    ] {
        let sink = Sink {
            fail_feed: true,
            ..Default::default()
        };
        run(
            vec![
                event("req-1", "text_delta", serde_json::json!({"delta": "a"})),
                terminal,
            ],
            &sink,
            false,
        )
        .await;
        assert!(sink.finals().is_empty());
    }
}

#[tokio::test]
async fn large_bursts_flush_in_bounded_batches() {
    let sink = Sink::default();
    let mut events: Vec<_> = (0..40)
        .map(|i| {
            event(
                "req-1",
                "text_delta",
                serde_json::json!({"delta": format!("{i}")}),
            )
        })
        .collect();
    events.push(event(
        "req-1",
        "chat_done",
        serde_json::json!({"full_response": "ok"}),
    ));
    run(events, &sink, false).await;
    assert_eq!(sink.events().len(), 40);
    assert!(
        sink.batches.lock().unwrap().len() >= 2,
        "a batch is flushed at 32 events"
    );
    assert_eq!(sink.finals(), ["ok"]);
}

#[tokio::test]
async fn the_loop_stops_when_the_session_is_cancelled_or_the_bus_closes() {
    let sink = Sink::default();
    let (tx, rx) = broadcast::channel(8);
    let session = tokio_util::sync::CancellationToken::new();
    session.cancel();
    run_channel_reply_loop(rx, "req-1".into(), "telegram".into(), session, &sink).await;
    assert!(sink.finals().is_empty());
    drop(tx);

    let sink = Sink::default();
    run(
        vec![event(
            "req-1",
            "text_delta",
            serde_json::json!({"delta": "x"}),
        )],
        &sink,
        true,
    )
    .await;
    assert!(sink.finals().is_empty());
}

#[tokio::test]
async fn a_lagging_receiver_skips_missed_events_and_still_finishes() {
    let sink = Sink::default();
    let (tx, rx) = broadcast::channel(2);
    for i in 0..6 {
        tx.send(event(
            "req-1",
            "text_delta",
            serde_json::json!({"delta": format!("{i}")}),
        ))
        .unwrap();
    }
    tx.send(event(
        "req-1",
        "chat_done",
        serde_json::json!({"full_response": "after lag"}),
    ))
    .unwrap();
    let session = tokio_util::sync::CancellationToken::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        run_channel_reply_loop(rx, "req-1".into(), "telegram".into(), session, &sink),
    )
    .await
    .expect("terminal event still arrives after a lag");
    assert_eq!(sink.finals(), ["after lag"]);
}
