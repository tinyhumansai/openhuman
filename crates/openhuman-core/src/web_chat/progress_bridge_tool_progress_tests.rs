use super::*;

/// A running tool's status line reaches the socket as `tool_progress`, tied to
/// the open call by its id, so the client can show the call is still alive.
#[tokio::test]
async fn forwards_tool_progress_for_the_running_call() {
    let mut events = super::super::super::event_bus::subscribe_web_channel_events();
    let thread_id = "thread-tool-progress";
    let tx = spawn_test_bridge(thread_id, "req-tool-progress");

    tx.send(AgentProgress::ToolCallProgress {
        call_id: "call-1".into(),
        message: "running 15s · 2.0 KB output".into(),
    })
    .await
    .unwrap();

    let event = recv_for_thread(&mut events, thread_id).await;
    assert_eq!(event.event, "tool_progress");
    assert_eq!(event.request_id, "req-tool-progress");
    assert_eq!(event.tool_call_id.as_deref(), Some("call-1"));
    assert_eq!(
        event.message.as_deref(),
        Some("running 15s · 2.0 KB output")
    );
    drop(tx);
}
