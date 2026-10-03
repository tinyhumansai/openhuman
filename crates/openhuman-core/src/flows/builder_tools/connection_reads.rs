//! Read-only connection tools: connectable toolkits and connection refs (ids/names only).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::config::Config;
use crate::flows::ops;
use tinytools::{PermissionLevel, Tool, ToolResult};

/// `list_connectable_toolkits`: read-only list of the Composio toolkits the
/// builder can wire, each tagged connected/unconnected — so the agent can steer
/// toolkit choice toward what's already connected (audit Phase 5, item 19).
pub struct ListConnectableToolkitsTool {
    config: Arc<Config>,
}

impl ListConnectableToolkitsTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
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

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        // The contract crate, not `memory::sync::composio::providers` (#5560).
        // That host shim is `pub use tinymemory_core::sync::composio::providers::*`
        // and the engine's `providers` module in turn re-exports this function
        // verbatim from `crate::integrations::composio::contract::scopes` — so the two paths
        // name the SAME item and this is a path change with no behaviour delta.
        // Naming the contract directly is what lets the shim's caller list
        // shrink to the sites that genuinely need the engine's registry and
        // curated catalogs.
        use crate::integrations::composio::contract::agent_ready_toolkits;
        tracing::debug!(target: "flows", "[flows] list_connectable_toolkits: listing toolkits + connected state (read-only) via the memory contract");
        let connected = ops::connected_toolkits(&self.config).await;
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

// ─────────────────────────────────────────────────────────────────────────────
// list_flow_connections — read-only: connection refs (ids/names only)
// ─────────────────────────────────────────────────────────────────────────────

/// `list_flow_connections`: read-only enumeration of the connection sources a
/// node's `connection_ref` can attach to (Composio connected accounts +
/// named HTTP credentials) — non-secret metadata only (ids / display labels
/// / kind / toolkit / scheme / platform_user_id), never secrets.
pub struct ListFlowConnectionsTool {
    config: Arc<Config>,
}

impl ListFlowConnectionsTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
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

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        tracing::debug!(target: "flows", "[flows] list_flow_connections: enumerating connection refs (read-only)");
        match ops::flows_list_connections(&self.config).await {
            Ok(outcome) => {
                let conns: Vec<Value> = outcome.value.iter().map(flow_connection_to_json).collect();
                Ok(ToolResult::success(serde_json::to_string_pretty(
                    &json!({ "connections": conns }),
                )?))
            }
            Err(e) => Ok(ToolResult::error(format!(
                "Failed to list flow connections: {e}"
            ))),
        }
    }
}

/// Render one [`crate::flows::types::FlowConnection`] as the
/// picker JSON shape the agent reads — ids/display/kind/toolkit/scheme plus
/// `platform_user_id` (the connected account's own member id, e.g. Slack
/// `U123ABC`, or `null` when no identity has synced yet). Never secret
/// material. A free function (rather than inline in `execute`) so the
/// mapping is unit-testable without a live Composio backend.
pub(super) fn flow_connection_to_json(c: &crate::flows::types::FlowConnection) -> Value {
    json!({
        "connection_ref": c.connection_ref,
        "kind": c.kind,
        "display": c.display,
        "toolkit": c.toolkit,
        "scheme": c.scheme,
        "platform_user_id": c.platform_user_id,
    })
}
