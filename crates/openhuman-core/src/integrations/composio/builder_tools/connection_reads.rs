//! Read-only connection tools: connectable toolkits and connection refs (ids/names only).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::config::Config;
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolResult, ToolRunContext};

/// Host supplied source for the flow editor's mixed connection picker.
#[async_trait]
pub trait FlowConnectionProvider: Send + Sync {
    async fn list(
        &self,
        dispatch: &crate::core::runtime::DispatchCtx,
        config: &Config,
    ) -> Result<Vec<Value>, String>;
}

/// Host source for the workflow editor's agent-ready toolkit list.
#[async_trait]
pub trait ConnectableToolkitProvider: Send + Sync {
    async fn connected(
        &self,
        dispatch: &crate::core::runtime::DispatchCtx,
        config: &Config,
    ) -> Vec<String>;
}

/// `list_connectable_toolkits`: read-only list of the Composio toolkits the
/// builder can wire, each tagged connected/unconnected — so the agent can steer
/// toolkit choice toward what's already connected (audit Phase 5, item 19).
pub struct ListConnectableToolkitsTool {
    config: Arc<Config>,
    provider: Arc<dyn ConnectableToolkitProvider>,
}

/// Read-only mixed connection picker backed by a host supplied provider.
pub struct ListFlowConnectionsTool {
    config: Arc<Config>,
    provider: Arc<dyn FlowConnectionProvider>,
}

impl ListFlowConnectionsTool {
    pub fn new(config: Arc<Config>, provider: Arc<dyn FlowConnectionProvider>) -> Self {
        Self { config, provider }
    }
}

#[async_trait]
impl Tool for ListFlowConnectionsTool {
    fn name(&self) -> &str {
        "list_flow_connections"
    }
    fn description(&self) -> &str {
        "List the connection sources a flow node's `connection_ref` can attach to: \
         Composio connected accounts and named HTTP credentials. Read-only; \
         returns only non-secret metadata — ids, display labels, kind, and \
         `toolkit`/`scheme` (never any secret). Each \
         Composio entry also carries `platform_user_id` — the connected \
         account's own member id (e.g. Slack `U123ABC`) — use it to wire a \
         self-targeted action like 'DM me' to that account instead of a \
         public channel. Use the `connection_ref` values verbatim on \
         tool_call / http_request nodes so the generated flow carries valid \
         connections."
    }
    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }
    fn external_effect(&self) -> bool {
        false
    }
    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, ToolCallOptions::default(), None)
            .await
    }
    async fn execute_with_context(
        &self,
        _args: Value,
        _options: ToolCallOptions,
        tool_context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let dispatch =
            crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(tool_context);
        tracing::debug!(target: "flows", "[flows] list_flow_connections: enumerating connection refs (read-only)");
        match self.provider.list(&dispatch, &self.config).await {
            Ok(connections) => Ok(ToolResult::success(serde_json::to_string_pretty(
                &json!({ "connections": connections }),
            )?)),
            Err(error) => Ok(ToolResult::error(format!(
                "Failed to list flow connections: {error}"
            ))),
        }
    }
}

impl ListConnectableToolkitsTool {
    pub fn new(config: Arc<Config>, provider: Arc<dyn ConnectableToolkitProvider>) -> Self {
        Self { config, provider }
    }
}

#[async_trait]
impl Tool for ListConnectableToolkitsTool {
    fn name(&self) -> &str {
        "list_connectable_toolkits"
    }

    fn description(&self) -> &str {
        "List the Composio toolkits available to wire into a tool_call/app_event, each flagged \
         `connected: true/false`. Read-only. Use it to prefer an ALREADY-connected toolkit when \
         several would work, and to tell the user which toolkits a proposed flow still needs \
         connecting. Returns a JSON array of { toolkit, connected }."
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    fn external_effect(&self) -> bool {
        false
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_context(
        &self,
        _args: Value,
        _options: ToolCallOptions,
        tool_context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let ctx = crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(tool_context);
        // The agent-ready list is the connector module's (`GetVocabulary`),
        // read through the host's curated snapshot.
        use crate::integrations::composio::contract::agent_ready_toolkits;
        tracing::debug!(target: "flows", "[flows] list_connectable_toolkits: listing toolkits + connected state (read-only) via the memory contract");
        let connected = self.provider.connected(&ctx, &self.config).await;
        let toolkits: Vec<Value> = agent_ready_toolkits()
            .into_iter()
            .map(|tk| {
                let tk_lc = tk.to_ascii_lowercase();
                json!({ "toolkit": tk_lc, "connected": connected.contains(&tk_lc) })
            })
            .collect();
        Ok(ToolResult::success(serde_json::to_string_pretty(
            &json!({ "toolkits": toolkits }),
        )?))
    }
}
