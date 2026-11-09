use super::*;
use crate::memory::test_fixtures::{bind_reference, config_in};

fn text(result: &ToolResult) -> String {
    result.text()
}

/// The memory activity a tool call raises reaches the chat its own turn is in:
/// the `MemoryRecalled` event carries the turn's thread and client, taken from
/// the run context the tool was dispatched with, and a call outside a chat turn
/// raises it unrouted. (A `learn` is queued and published off the turn, so it
/// is never routed.)
#[tokio::test]
async fn memory_activity_carries_the_calling_turns_chat() {
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().unwrap().receiver();
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let _engine = bind_reference(&config);
    let tool = MemoryTool::new(Arc::new(config.clone()));

    for memory_text in ["routed marker question", "unrouted marker question"] {
        let learned = tool
            .execute(json!({"action": "learn", "text": memory_text}))
            .await
            .unwrap();
        assert!(!learned.is_error, "{}", text(&learned));
    }
    crate::memory::tool_writes::drain(&config).await;

    let context = crate::agent::tinyagents::host::run_context::tool_context_with_approval(
        crate::security::approval::ApprovalTurnContext::chat(
            crate::security::approval::ApprovalChatContext {
                thread_id: "thread-activity".into(),
                client_id: "client-activity".into(),
                request_id: None,
            },
        ),
    );
    let routed = tool
        .execute_with_context(
            json!({"action": "recall", "question": "routed marker question"}),
            ToolCallOptions::default(),
            Some(&context),
        )
        .await
        .unwrap();
    assert!(!routed.is_error, "{}", text(&routed));
    let unrouted = tool
        .execute(json!({"action": "recall", "question": "unrouted marker question"}))
        .await
        .unwrap();
    assert!(!unrouted.is_error, "{}", text(&unrouted));
    let expected_routed_hits = serde_json::from_str::<Value>(&text(&routed)).unwrap()["items"]
        .as_u64()
        .unwrap();
    let expected_unrouted_hits = serde_json::from_str::<Value>(&text(&unrouted)).unwrap()["items"]
        .as_u64()
        .unwrap();
    assert!(
        expected_routed_hits > 0,
        "the seeded memory must be recalled"
    );
    assert!(
        expected_unrouted_hits > 0,
        "the seeded memory must be recalled"
    );

    // The bus is process-wide: match our own events by the question text.
    let mut seen = std::collections::HashMap::new();
    while seen.len() < 2 {
        match tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await {
            Ok(Some(DomainEvent::MemoryRecalled {
                query,
                thread_id,
                client_id,
                hit_count,
            })) if query.contains("marker question") => {
                seen.insert(query, (thread_id, client_id, hit_count));
            }
            Ok(Some(_)) => continue,
            other => panic!("expected both MemoryRecalled events, got {other:?}"),
        }
    }
    assert_eq!(
        seen["routed marker question"],
        (
            Some("thread-activity".into()),
            Some("client-activity".into()),
            expected_routed_hits as usize,
        )
    );
    assert_eq!(
        seen["unrouted marker question"],
        (None, None, expected_unrouted_hits as usize)
    );
}
