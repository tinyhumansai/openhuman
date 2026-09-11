//! Event bus subscriber for the MCP clients domain.
//!
//! Logs lifecycle events (`McpServer*`, `McpClientToolExecuted`) for
//! observability. No side effects are performed here — domain logic lives
//! in `ops.rs` and `connections.rs`.

use async_trait::async_trait;

use crate::core::events::DomainEvent;
use tinybus::EventHandler;

/// Subscribes to `McpClient` domain events and emits structured log lines.
pub struct McpClientEventSubscriber;

#[async_trait]
impl EventHandler<DomainEvent> for McpClientEventSubscriber {
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
    use crate::core::bus::BUS;
    use std::sync::Arc;
    let sub = Arc::new(McpClientEventSubscriber);
    if BUS.subscribe(sub).is_none() {
        tracing::warn!("[mcp-client] event bus not initialized; subscriber not registered");
    }
}

#[cfg(test)]
#[path = "bus_tests.rs"]
mod tests;
