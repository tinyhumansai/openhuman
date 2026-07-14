//! Round20 focused raw coverage for channel bus and presentation paths.
//!
//! Uses debug-only seams plus in-memory web-channel events. No external
//! channel credentials, providers, or network services are required.

use std::time::Duration;

use openhuman_core::core::event_bus::{DomainEvent, EventHandler};
use openhuman_core::openhuman::agent_memory::memory_loader::MemoryCitation;
use openhuman_core::openhuman::channels::bus::ChannelInboundSubscriber;
use openhuman_core::openhuman::channels::providers::web::presentation::test_support as presentation_test_support;
use openhuman_core::openhuman::channels::providers::web::{
    subscribe_web_channel_events, test_support as web_test_support,
};
use serde_json::json;
use tokio::time::timeout;

#[tokio::test]
async fn presentation_segments_text_and_delivers_single_bubble_with_citations() {
    assert_eq!(
        presentation_test_support::segment_for_delivery_for_test("  Hello there.  "),
        vec!["Hello there.".to_string()]
    );
    assert_eq!(
        presentation_test_support::segment_for_delivery_for_test(
            "Here is code:\n\n```rust\nfn main() {}\n```\n\nKeep it together."
        )
        .len(),
        1
    );
    assert!(presentation_test_support::is_structured_content_for_test(
        "# Heading\n- First\n- Second"
    ));
    assert_eq!(presentation_test_support::segment_delay_for_test(""), 500);

    let citation = MemoryCitation {
        id: "mem-1".to_string(),
        key: "project".to_string(),
        namespace: Some("test".to_string()),
        score: Some(0.91),
        timestamp: "2026-05-29T00:00:00Z".to_string(),
        snippet: "OpenHuman channel presentation citation.".to_string(),
    };

    let mut rx = subscribe_web_channel_events();
    presentation_test_support::deliver_response_for_test(
        "round20-client",
        "round20-thread",
        "round20-single",
        "Short final answer.",
        "",
        std::slice::from_ref(&citation),
    )
    .await;

    // `subscribe_web_channel_events` is a process-global bus shared with the
    // other presentation tests, which run concurrently and emit their own
    // `chat_segment`/`chat_done` events. Filter to this delivery's request id so
    // a sibling's segment event can't be mistaken for our single bubble.
    let event = timeout(Duration::from_secs(5), async {
        loop {
            let event = rx.recv().await.expect("single bubble event");
            if event.request_id == "round20-single" {
                break event;
            }
        }
    })
    .await
    .expect("single bubble event timeout");
    assert_eq!(event.event, "chat_done");
    assert_eq!(event.full_response.as_deref(), Some("Short final answer."));
    assert_eq!(event.reaction_emoji, None);
    assert!(event
        .citations
        .expect("citations")
        .to_string()
        .contains("mem-1"));
}

#[tokio::test]
async fn presentation_delivers_segment_events_then_deduping_done_event() {
    let response = [
        "First paragraph has enough natural language content to stand alone as a separate chat bubble.",
        "Second paragraph also contains enough prose to exercise segmented delivery and delay calculation.",
        "Third paragraph ensures the final chat_done event carries the complete response for deduplication.",
    ]
    .join("\n\n");
    let segments = presentation_test_support::segment_for_delivery_for_test(&response);
    assert!(segments.len() >= 2, "expected segmented delivery");

    let mut rx = subscribe_web_channel_events();
    presentation_test_support::deliver_response_for_test(
        "round20-client",
        "round20-thread",
        "round20-segmented",
        &response,
        "",
        &[],
    )
    .await;

    let mut seen_segments = 0_u32;
    let final_event = timeout(Duration::from_secs(10), async {
        loop {
            let event = rx.recv().await.expect("presentation event");
            if event.request_id != "round20-segmented" {
                continue;
            }
            match event.event.as_str() {
                "chat_segment" => {
                    assert_eq!(event.segment_total, Some(segments.len() as u32));
                    assert_eq!(event.segment_index, Some(seen_segments));
                    assert!(event.full_response.as_deref().unwrap_or("").len() >= 40);
                    seen_segments += 1;
                }
                "chat_done" => break event,
                other => panic!("unexpected presentation event {other}"),
            }
        }
    })
    .await
    .expect("segmented delivery timeout");

    assert_eq!(seen_segments, segments.len() as u32);
    assert_eq!(final_event.segment_total, Some(segments.len() as u32));
    assert_eq!(
        final_event.full_response.as_deref(),
        Some(response.as_str())
    );
}

#[tokio::test]
async fn channel_inbound_subscriber_handles_forced_web_error_without_external_services() {
    let subscriber = ChannelInboundSubscriber::new();
    assert_eq!(subscriber.name(), "channel::inbound_handler");
    assert_eq!(subscriber.domains(), Some(&["channel"][..]));

    web_test_support::set_forced_run_chat_task_error_for_test(Some(
        "openrouter API error (429 Too Many Requests): Retry-After: 3",
    ))
    .await;

    timeout(
        Duration::from_secs(10),
        subscriber.handle(&DomainEvent::ChannelInboundMessage {
            event_name: "discord:message".to_string(),
            channel: "discord:guild-1".to_string(),
            message: "Please summarize the thread.".to_string(),
            sender: Some("user-a".to_string()),
            reply_target: Some("channel-a".to_string()),
            thread_ts: Some("1700000000.001".to_string()),
            raw_data: json!({ "round": 20 }),
        }),
    )
    .await
    .expect("inbound subscriber should finish after forced web error");

    web_test_support::set_forced_run_chat_task_error_for_test(None).await;
}

#[tokio::test]
async fn channel_inbound_subscriber_ignores_unrelated_events() {
    timeout(
        Duration::from_secs(2),
        ChannelInboundSubscriber::default().handle(&DomainEvent::SystemStartup {
            component: "round20".to_string(),
        }),
    )
    .await
    .expect("unrelated event should return immediately");
}
