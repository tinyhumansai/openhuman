//! Per-action Composio tool wrapper.
//!
//! A [`ComposioActionTool`] is a [`Tool`] that represents exactly one
//! Composio action (e.g. `GMAIL_SEND_EMAIL`). It holds the action's
//! name, description, and parameter JSON schema so the LLM's native
//! tool-calling path can validate arguments before they hit the wire.
//!
//! These are constructed **dynamically at spawn time** by the sub-agent
//! runner when `integrations_agent` is spawned with a `toolkit` argument —
//! one tool per action in the chosen toolkit. The generic
//! [`ComposioExecuteTool`](super::tools::ComposioExecuteTool) dispatcher
//! is deliberately excluded from `integrations_agent`'s tool list in that
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
use super::providers::ToolScope;
use super::tools::resolve_action_scope;
use crate::openhuman::agent::harness::current_sandbox_mode;
use crate::openhuman::agent::harness::definition::SandboxMode;
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
        // Agent-level sandbox gate (issue #685, CodeRabbit follow-up on
        // PR #904) — mirrors the check in
        // [`super::tools::ComposioExecuteTool::execute`] so a read-only
        // agent cannot slip a mutating call through the per-action
        // surface. The dispatcher path (`composio_execute`) and this
        // per-action path are the only two routes to the Composio
        // backend; both must honour the same invariant. Today no
        // read-only agent spawns per-action tools (only
        // `integrations_agent` registers them and it is
        // `sandbox_mode = "none"`), so this is strict defense-in-depth
        // for any future configuration that pairs the two.
        if matches!(current_sandbox_mode(), Some(SandboxMode::ReadOnly)) {
            let scope = resolve_action_scope(&self.action_name).await;
            if matches!(scope, ToolScope::Write | ToolScope::Admin) {
                tracing::info!(
                    tool = %self.action_name,
                    scope = scope.as_str(),
                    "[composio][sandbox] per-action execute blocked: agent is read-only, action is {}",
                    scope.as_str()
                );
                return Ok(ToolResult::error(format!(
                    "{}: action is classified `{}` and is refused because the calling \
                     agent is in strict read-only mode. Only `read`-scoped actions are \
                     available to this agent.",
                    self.action_name,
                    scope.as_str()
                )));
            }
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::harness::with_current_sandbox_mode;
    use crate::openhuman::integrations::IntegrationClient;
    use std::sync::Arc;

    /// Build a `ComposioClient` whose backend is the loopback dead-drop
    /// used by the tests in `composio/tools.rs`. The sandbox gate runs
    /// *before* any HTTP call, so these tests never reach the network.
    fn fake_client() -> ComposioClient {
        let inner =
            IntegrationClient::new("http://127.0.0.1:0".to_string(), "test-token".to_string());
        ComposioClient::new(Arc::new(inner))
    }

    fn error_text(result: &ToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|c| match c {
                crate::openhuman::tools::traits::ToolContent::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[tokio::test]
    async fn sandbox_read_only_blocks_per_action_write_call() {
        let t = ComposioActionTool::new(
            fake_client(),
            "GMAIL_SEND_EMAIL".to_string(),
            "send a gmail message".to_string(),
            None,
        );
        let result = with_current_sandbox_mode(SandboxMode::ReadOnly, async {
            t.execute(serde_json::json!({})).await.unwrap()
        })
        .await;
        assert!(
            result.is_error,
            "per-action Write under read-only must error"
        );
        let msg = error_text(&result);
        assert!(msg.contains("strict read-only"), "got: {msg}");
        assert!(msg.contains("`write`"), "got: {msg}");
    }

    #[tokio::test]
    async fn sandbox_read_only_blocks_per_action_admin_call() {
        let t = ComposioActionTool::new(
            fake_client(),
            "GMAIL_DELETE_EMAIL".to_string(),
            "destructive".to_string(),
            None,
        );
        let result = with_current_sandbox_mode(SandboxMode::ReadOnly, async {
            t.execute(serde_json::json!({})).await.unwrap()
        })
        .await;
        assert!(result.is_error);
        let msg = error_text(&result);
        assert!(msg.contains("`admin`"), "got: {msg}");
    }

    #[tokio::test]
    async fn sandbox_unset_leaves_per_action_execute_to_downstream() {
        // Outside any `with_current_sandbox_mode` scope the task-local
        // is `None` and the gate is a no-op. The downstream HTTP call
        // still fails (loopback :0), but never with the sandbox text.
        let t = ComposioActionTool::new(
            fake_client(),
            "GMAIL_SEND_EMAIL".to_string(),
            "send".to_string(),
            None,
        );
        let result = t.execute(serde_json::json!({})).await.unwrap();
        let msg = error_text(&result);
        assert!(
            !msg.contains("strict read-only"),
            "unset sandbox must never trigger the gate, got: {msg}"
        );
    }
}
