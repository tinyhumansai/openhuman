use super::*;

#[tokio::test]
async fn subscriber_ignores_unrelated_events() {
    let sub = McpClientEventSubscriber;
    // Should not panic on unrelated events
    sub.handle(&DomainEvent::SystemStartup {
        component: "test".to_string(),
    })
    .await;
}

#[tokio::test]
async fn subscriber_handles_mcp_installed_event() {
    let sub = McpClientEventSubscriber;
    sub.handle(&DomainEvent::McpServerInstalled {
        server_id: "srv-1".to_string(),
        qualified_name: "@test/server".to_string(),
    })
    .await;
}

#[tokio::test]
async fn subscriber_handles_mcp_connected_event() {
    let sub = McpClientEventSubscriber;
    sub.handle(&DomainEvent::McpServerConnected {
        server_id: "srv-1".to_string(),
        tool_count: 3,
    })
    .await;
}

#[tokio::test]
async fn subscriber_handles_mcp_disconnected_event() {
    let sub = McpClientEventSubscriber;
    sub.handle(&DomainEvent::McpServerDisconnected {
        server_id: "srv-1".to_string(),
        reason: Some("user request".to_string()),
    })
    .await;
}

#[tokio::test]
async fn subscriber_handles_tool_executed_event() {
    let sub = McpClientEventSubscriber;
    sub.handle(&DomainEvent::McpClientToolExecuted {
        server_id: "srv-1".to_string(),
        tool_name: "search".to_string(),
        success: true,
        elapsed_ms: 42,
    })
    .await;
}
