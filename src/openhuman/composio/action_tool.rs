//! Per-action Composio tool wrapper.
//!
//! A [`ComposioActionTool`] is a [`Tool`] that represents exactly one
//! Composio action (e.g. `GMAIL_SEND_EMAIL`). It holds the action's
//! name, description, and parameter JSON schema so the LLM's native
//! tool-calling path can validate arguments before they hit the wire.
//!
//! These are constructed **dynamically at spawn time** by the sub-agent
//! runner when `skills_agent` is spawned with a `toolkit` argument —
//! one tool per action in the chosen toolkit. The generic
//! [`ComposioExecuteTool`](super::tools::ComposioExecuteTool) dispatcher
//! is deliberately excluded from `skills_agent`'s tool list in that
//! path so the model doesn't see two ways to call the same action.
//!
//! Lifetime: these tools live for the duration of a single sub-agent
//! spawn. The underlying [`ComposioClient`] is cheap to clone (it
//! wraps an `Arc<IntegrationClient>` internally), so each tool holds
//! its own owned clone and calls `client.execute_tool` directly when
//! invoked — no config reload or client rebuild on the hot path.

use async_trait::async_trait;
use serde_json::Value;

use super::client::ComposioClient;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCategory, ToolResult};

/// A single Composio action exposed as a first-class tool.
pub struct ComposioActionTool {
    client: ComposioClient,
    /// Action slug as-shipped to Composio, e.g. `"GMAIL_SEND_EMAIL"`.
    action_name: String,
    /// Human-readable description from the Composio tool-list response.
    description: String,
    /// Full JSON schema for the action's parameters. Falls back to
    /// `{"type":"object"}` when the upstream response omits it so the
    /// LLM still gets a valid (if loose) shape.
    parameters: Value,
}

impl ComposioActionTool {
    pub fn new(
        client: ComposioClient,
        action_name: String,
        description: String,
        parameters: Option<Value>,
    ) -> Self {
        let parameters = parameters.unwrap_or_else(|| serde_json::json!({"type": "object"}));
        Self {
            client,
            action_name,
            description,
            parameters,
        }
    }
}

#[async_trait]
impl Tool for ComposioActionTool {
    fn name(&self) -> &str {
        &self.action_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        self.parameters.clone()
    }

    fn permission_level(&self) -> PermissionLevel {
        // Conservative default: many actions mutate external state
        // (send mail, create issues, modify calendars). Match
        // ComposioExecuteTool's write-level treatment so channel
        // permission caps behave identically whether the model goes
        // through the dispatcher or a per-action tool.
        PermissionLevel::Write
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Skill
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let started = std::time::Instant::now();
        let res = self
            .client
            .execute_tool(&self.action_name, Some(args))
            .await;
        let elapsed_ms = started.elapsed().as_millis() as u64;

        match res {
            Ok(resp) => {
                crate::core::event_bus::publish_global(
                    crate::core::event_bus::DomainEvent::ComposioActionExecuted {
                        tool: self.action_name.clone(),
                        success: resp.successful,
                        error: resp.error.clone(),
                        cost_usd: resp.cost_usd,
                        elapsed_ms,
                    },
                );
                Ok(ToolResult::success(
                    serde_json::to_string(&resp).unwrap_or_else(|_| "{}".into()),
                ))
            }
            Err(e) => {
                crate::core::event_bus::publish_global(
                    crate::core::event_bus::DomainEvent::ComposioActionExecuted {
                        tool: self.action_name.clone(),
                        success: false,
                        error: Some(e.to_string()),
                        cost_usd: 0.0,
                        elapsed_ms,
                    },
                );
                Ok(ToolResult::error(format!("{}: {e}", self.action_name)))
            }
        }
    }
}
