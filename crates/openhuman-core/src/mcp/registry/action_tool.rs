//! Agent tools for every action on the user's installed (`mcp.json`) servers.
//!
//! The tools themselves are `tinymcp::tools::McpServerTool`s: named
//! `mcp_<server>_<tool>`, deferred so `tool_search` finds them, described and
//! schema'd from sanitized remote text. What stays here is this application's
//! policy, applied through [`InstalledServerInvoker`]:
//!
//! - remote definitions pass the prompt-injection scan
//!   ([`super::tools_safe_for_agent`]) before they become tools, and again
//!   against the live list at call time;
//! - a call reaches only a server that is connected now — the tool list may
//!   come from the persistent cache, which is not an authorization;
//! - every call publishes `McpClientToolExecuted`.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use serde_json::Value;
use tinymcp::tools::{McpServerTool, McpToolInvoker, McpToolSource};
use tinymcp_bus::McpToolResult;
use tinytools::Tool;

use crate::config::Config;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::mcp::host;

use super::connections;
use super::types::ConnectedServerOverview;

/// The name an installed server's tool is exposed under.
///
/// `mcp_<server>_<tool>_<digest>`: the server part from its qualified name,
/// plus a short digest of the install and tool so the name never changes
/// when another server with the same slug comes or goes; see
/// `tinymcp::tools::naming`.
#[must_use]
pub fn searchable_name(server_id: &str, qualified_name: &str, tool_name: &str) -> String {
    tinymcp::tools::naming::disambiguated_tool_name(server_id, qualified_name, tool_name)
}

/// One deferred tool per action of each installed server in `servers`.
///
/// `servers` may come from the persistent tool cache
/// (`McpRegistry::cached_overview`), so tools appear before their server has
/// finished connecting; the invoker refuses a call until it has.
pub fn deferred_connected_tools(
    config: Arc<Config>,
    servers: &[ConnectedServerOverview],
) -> Vec<Box<dyn Tool>> {
    server_tools(config, servers)
        .into_iter()
        .map(|tool| Box::new(tool) as Box<dyn Tool>)
        .collect()
}

/// [`deferred_connected_tools`], plus a copy of any tool under the pre-readable
/// hashed name when `recorded` says the conversation was sent that name.
///
/// A resumed thread replays the tool names it recorded, so a thread started
/// before readable names keeps working without any file on disk changing.
pub fn deferred_connected_tools_with_legacy(
    config: Arc<Config>,
    servers: &[ConnectedServerOverview],
    recorded: &HashSet<String>,
) -> Vec<Box<dyn Tool>> {
    let tools = server_tools(config, servers);
    let aliases: Vec<McpServerTool> = tools
        .iter()
        .filter_map(|tool| {
            let legacy = tool.legacy_name();
            (recorded.contains(&legacy) && legacy != tool.name())
                .then(|| tool.clone().renamed(legacy))
        })
        .collect();
    if !aliases.is_empty() {
        tracing::debug!(
            aliases = aliases.len(),
            "[mcp] restoring recorded tools under their earlier names"
        );
    }
    tools
        .into_iter()
        .chain(aliases)
        .map(|tool| Box::new(tool) as Box<dyn Tool>)
        .collect()
}

fn server_tools(config: Arc<Config>, servers: &[ConnectedServerOverview]) -> Vec<McpServerTool> {
    let sources: Vec<McpToolSource> = servers
        .iter()
        .map(|server| {
            let mut source = McpToolSource::from_overview(server);
            source.tools = super::tools_safe_for_agent(&server.server_id, server.tools.clone());
            source
        })
        .collect();
    let invoker: Arc<dyn McpToolInvoker> = Arc::new(InstalledServerInvoker { config });
    tinymcp::tools::tools_for(&sources, &invoker)
}

/// Calls an installed server's tool under this application's policy.
#[derive(Debug)]
pub struct InstalledServerInvoker {
    config: Arc<Config>,
}

impl InstalledServerInvoker {
    /// An invoker over `config`'s workspace.
    #[must_use]
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl McpToolInvoker for InstalledServerInvoker {
    async fn invoke(
        &self,
        server_id: &str,
        tool: &str,
        arguments: Value,
    ) -> tinymcp::Result<McpToolResult> {
        let not_connected = || tinymcp::Error::NotConnected {
            server: server_id.to_string(),
        };
        let live = connections::server_tools_for_config(&self.config, server_id)
            .await
            .ok_or_else(not_connected)?;
        let safe = super::tools_safe_for_agent(server_id, live);
        if !safe.iter().any(|candidate| candidate.name == tool) {
            tracing::debug!(
                server_id,
                tool,
                "[mcp] tool is no longer offered by the live server"
            );
            return Err(tinymcp::Error::ToolNotAllowed {
                server: server_id.to_string(),
                tool: tool.to_string(),
            });
        }
        let start = Instant::now();
        let result = async {
            let client = crate::mcp::module_client::McpConfiguredClient::for_config(&self.config)
                .map_err(|error| {
                tracing::debug!(?error, server_id, "[mcp] no module for workspace");
                not_connected()
            })?;
            let outcome: tinymcp_bus::ToolCallOutcome = client
                .call(
                    &host::client_config(&self.config),
                    tinymcp_bus::names::methods::TOOL_CALL,
                    (server_id, tool, arguments),
                )
                .await
                .map_err(|error| {
                    tracing::debug!(?error, server_id, tool, "[mcp] module tool call failed");
                    not_connected()
                })?;
            Ok::<_, tinymcp::Error>(outcome.rendered)
        }
        .await;
        let elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        BUS.publish(DomainEvent::McpClientToolExecuted {
            server_id: server_id.to_string(),
            tool_name: tool.to_string(),
            success: result.is_ok(),
            elapsed_ms,
        });
        tracing::debug!(
            server_id,
            tool,
            elapsed_ms,
            ok = result.is_ok(),
            "[mcp] installed server tool call"
        );
        result
    }
}

#[cfg(test)]
#[path = "action_tool_tests.rs"]
mod tests;
