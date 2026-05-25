//! Event bus subscriber for the MCP clients domain.
//!
//! Logs lifecycle events (`McpServer*`, `McpClientToolExecuted`) for
//! observability. No side effects are performed here — domain logic lives
//! in `ops.rs` and `connections.rs`.

use async_trait::async_trait;

use crate::core::event_bus::{DomainEvent, EventHandler};

/// Subscribes to `McpClient` domain events and emits structured log lines.
pub struct McpClientEventSubscriber;

#[async_trait]
impl EventHandler for McpClientEventSubscriber {
    fn name(&self) -> &str {
        "mcp_client::lifecycle"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["mcp_client"])
    }

    async fn handle(&self, event: &DomainEvent) {
        match event {
            DomainEvent::McpServerInstalled {
                server_id,
                qualified_name,
            } => {
                tracing::info!(
                    server_id = %server_id,
                    qualified_name = %qualified_name,
                    "[mcp-client] server installed"
                );
            }

            DomainEvent::McpServerConnected {
                server_id,
                tool_count,
            } => {
                tracing::info!(
                    server_id = %server_id,
                    tool_count = %tool_count,
                    "[mcp-client] server connected"
                );
            }

            DomainEvent::McpServerDisconnected { server_id, reason } => {
                tracing::info!(
                    server_id = %server_id,
                    reason = ?reason,
                    "[mcp-client] server disconnected"
                );
            }

            DomainEvent::McpClientToolExecuted {
                server_id,
                tool_name,
                success,
                elapsed_ms,
            } => {
                tracing::debug!(
                    server_id = %server_id,
                    tool_name = %tool_name,
                    success = %success,
                    elapsed_ms = %elapsed_ms,
                    "[mcp-client] tool executed"
                );
            }

            _ => {}
        }
    }
}

/// Register the MCP client event subscriber at startup.
///
/// Call this from wherever other domain subscribers are registered
/// (e.g. alongside `CronDeliverySubscriber::new(...)` in core startup).
pub fn init() {
    use crate::core::event_bus::subscribe_global;
    use std::sync::Arc;
    let sub = Arc::new(McpClientEventSubscriber);
    if subscribe_global(sub).is_none() {
        tracing::warn!("[mcp-client] event bus not initialized; subscriber not registered");
    }
}

#[cfg(test)]
mod tests {
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
}
