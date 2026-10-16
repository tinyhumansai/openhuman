//! The `composio_authorize` agent tool.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::live_config::live_composio_config;
use super::redact::redact_composio_outcome;
use crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool;
use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolCategory, ToolResult, ToolRunContext};

use super::super::client::{resolve_composio_route, ComposioRoute};
use super::super::module_client::{self as connectors};
use super::super::types::{ComposioAuthorizeRequest, ComposioAuthorizeResponse};
use tinyconnectors_bus::names::methods;

pub struct ComposioAuthorizeTool {
    /// Held instead of a pre-resolved route so the
    /// [`crate::config::ComposioConfig::mode`] toggle is
    /// honoured on every call (#1710).
    config: Arc<Config>,
}

impl ComposioAuthorizeTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ComposioAuthorizeTool {
    fn name(&self) -> &str {
        "composio_authorize"
    }
    fn description(&self) -> &str {
        "Begin an OAuth handoff for a Composio toolkit. Returns a `connectUrl` \
         the user must open in a browser to authorize the integration, plus the \
         resulting `connectionId`. The toolkit must be in the backend allowlist."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "toolkit": {
                    "type": "string",
                    "description": "Toolkit slug, e.g. 'gmail' or 'notion'."
                }
            },
            "required": ["toolkit"],
            "additionalProperties": false
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Workflow
    }
    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, ToolCallOptions::default(), None)
            .await
    }
    async fn execute_with_context(
        &self,
        args: Value,
        _options: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let ctx = dispatch_ctx_for_tool(context);
        let outcome = Box::pin(self.execute_unredacted(&ctx, args)).await;
        redact_composio_outcome(&self.config, outcome)
    }
}

impl ComposioAuthorizeTool {
    async fn execute_unredacted(
        &self,
        ctx: &DispatchCtx,
        args: Value,
    ) -> anyhow::Result<ToolResult> {
        let toolkit = args
            .get("toolkit")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if toolkit.is_empty() {
            return Ok(ToolResult::error(
                "composio_authorize: 'toolkit' is required",
            ));
        }
        tracing::debug!(toolkit = %toolkit, "[composio] tool authorize.execute");
        // Resolve per call so a live mode toggle is honoured. In
        // direct mode the OAuth handoff is performed by the user's
        // personal Composio tenant via app.composio.dev rather than
        // the backend's composio authorize
        // route, so we refuse this verb explicitly instead of
        // silently routing through the wrong tenant.
        let live_config = match live_composio_config(self.config.as_ref()).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "[composio] tool: load_config failed");
                return Ok(ToolResult::error(format!(
                    "composio: failed to load live config: {e}"
                )));
            }
        };
        match resolve_composio_route(&live_config) {
            Ok(ComposioRoute::Backend) => {
                tracing::debug!("[composio] authorize.execute: backend variant");
            }
            Ok(ComposioRoute::Direct(_)) => {
                tracing::info!(
                    toolkit = %toolkit,
                    "[composio-direct] authorize.execute: direct mode active — \
                     refusing backend OAuth handoff. Connect this toolkit via \
                     app.composio.dev for the personal Composio tenant."
                );
                return Ok(ToolResult::error(format!(
                    "composio_authorize: direct mode is active. Connect `{toolkit}` \
                     through your personal Composio account at app.composio.dev \
                     instead of the backend OAuth flow."
                )));
            }
            Err(e) => {
                return Ok(ToolResult::error(format!("composio_authorize failed: {e}")));
            }
        };
        let request = ComposioAuthorizeRequest {
            toolkit: toolkit.clone(),
            extra_params: None,
        };
        match connectors::call::<_, ComposioAuthorizeResponse>(
            &live_config,
            methods::AUTHORIZE,
            request,
        )
        .await
        {
            Ok(resp) => {
                crate::core::bus::BUS.publish(
                    crate::core::events::DomainEvent::ComposioConnectionCreated {
                        toolkit: toolkit.clone(),
                        connection_id: resp.connection_id.clone(),
                        connect_url: resp.connect_url.clone(),
                        tenant: ctx.event_tenant(),
                    },
                );
                Ok(ToolResult::success(format!(
                    "Open this URL to connect {toolkit}: {}\n(connectionId: {})",
                    resp.connect_url, resp.connection_id
                )))
            }
            Err(e) => Ok(ToolResult::error(format!("composio_authorize failed: {e}"))),
        }
    }
}
