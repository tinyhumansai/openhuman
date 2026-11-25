use super::*;

/// A running tool's progress report reaches the web channel as
/// `ToolCallProgress` for the same call; an update with no message does not.
#[tokio::test]
async fn bridge_forwards_tool_progress_with_a_message() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let bridge = OpenhumanEventBridge::new(Some(tx), "mock-model", 10);
    let sink = EventSink::new();
    sink.subscribe(bridge.clone());

    sink.emit(AgentEvent::ToolProgressDetail {
        call_id: "c1".into(),
        message: "running 4m 12s · 38.2 KB output".to_string(),
        fraction: None,
        partial: None,
    });
    sink.emit(AgentEvent::ToolProgressDetail {
        call_id: "c1".into(),
        message: String::new(),
        fraction: Some(0.5),
        partial: None,
    });

    let mut progress = Vec::new();
    while let Ok(p) = rx.try_recv() {
        if let AgentProgress::ToolCallProgress { call_id, message } = p {
            progress.push((call_id, message));
        }
    }
    assert_eq!(
        progress,
        vec![(
            "c1".to_string(),
            "running 4m 12s · 38.2 KB output".to_string()
        )]
    );
}
