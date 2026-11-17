//! Read-only checks over MCP clients that are already open.

use std::sync::Arc;

use async_trait::async_trait;

use crate::config::McpServerConfig;
use crate::doctor::contract::{ComponentId, DoctorCheck, DoctorCtx, DoctorTier, Finding};

use super::canonical_segment;

#[async_trait]
trait McpStatusProbe: Send + Sync {
    async fn statuses(
        &self,
        config: &crate::config::Config,
    ) -> Result<Option<Vec<tinymcp_bus::ConnStatus>>, McpStatusFailure>;
}

#[derive(Clone, Copy)]
enum McpStatusFailure {
    Unavailable,
    Failed,
}

struct ExistingClientProbe;

#[async_trait]
impl McpStatusProbe for ExistingClientProbe {
    async fn statuses(
        &self,
        config: &crate::config::Config,
    ) -> Result<Option<Vec<tinymcp_bus::ConnStatus>>, McpStatusFailure> {
        #[cfg(feature = "mcp")]
        {
            crate::mcp::module_client::McpConfiguredClient::existing_status_for_config(config)
                .await
                .map_err(|error| match error {
                    tinybus::module::ModuleCallError::Unavailable => McpStatusFailure::Unavailable,
                    _ => McpStatusFailure::Failed,
                })
        }
        #[cfg(not(feature = "mcp"))]
        {
            let _ = config;
            Err(McpStatusFailure::Unavailable)
        }
    }
}

pub(super) fn for_context(ctx: &DoctorCtx) -> Vec<Arc<dyn DoctorCheck>> {
    checks_for_servers(
        &ctx.config().mcp_client.servers,
        Arc::new(ExistingClientProbe),
    )
    .into_iter()
    .map(|check| Arc::new(check) as Arc<dyn DoctorCheck>)
    .collect()
}

fn checks_for_servers(
    servers: &[McpServerConfig],
    probe: Arc<dyn McpStatusProbe>,
) -> Vec<McpServerCheck> {
    let mut indexes = std::collections::HashMap::<String, usize>::new();
    let mut checks: Vec<McpServerCheck> = Vec::new();
    for server in servers {
        let check = McpServerCheck::configured(server, Arc::clone(&probe));
        if let Some(index) = indexes.get(&check.id).copied() {
            checks[index].duplicate_definitions = true;
        } else {
            indexes.insert(check.id.clone(), checks.len());
            checks.push(check);
        }
    }
    checks
}

struct McpServerCheck {
    id: String,
    server_id: String,
    enabled: bool,
    duplicate_definitions: bool,
    probe: Arc<dyn McpStatusProbe>,
}

impl McpServerCheck {
    fn configured(server: &McpServerConfig, probe: Arc<dyn McpStatusProbe>) -> Self {
        Self {
            id: format!("mcp/{}", canonical_segment(&server.name)),
            server_id: server.name.clone(),
            enabled: server.enabled,
            duplicate_definitions: false,
            probe,
        }
    }

    #[cfg(test)]
    fn with_probe(server: &McpServerConfig, probe: Arc<dyn McpStatusProbe>) -> Self {
        Self {
            id: format!("mcp/{}", canonical_segment(&server.name)),
            server_id: server.name.clone(),
            enabled: server.enabled,
            duplicate_definitions: false,
            probe,
        }
    }

    async fn finding(&self, ctx: &DoctorCtx) -> Finding {
        if !ctx.config().components.mcp
            || !ctx.config().modules.enabled
            || !ctx.config().mcp_client.enabled
        {
            return Finding::skipped(
                &self.id,
                "MCP is disabled in the selected configuration.",
                "Enable MCP in the selected configuration to check server connections.",
            );
        }
        if self.duplicate_definitions {
            return Finding::error(
                &self.id,
                "Multiple MCP server definitions use a duplicate identifier; their connection state is ambiguous.",
                "Remove or rename duplicate entries in mcp_client.servers so each server name is unique.",
            );
        }
        if !self.enabled {
            return Finding::skipped(
                &self.id,
                "The configured MCP server is disabled.",
                "Enable the server in MCP settings to check its connection.",
            );
        }
        let statuses = match self.probe.statuses(ctx.config()).await {
            Ok(Some(statuses)) => statuses,
            Ok(None) | Err(McpStatusFailure::Unavailable) => {
                return Finding::skipped(
                    &self.id,
                    "No matching MCP runtime is already open for this configuration.",
                    "Open the MCP runtime, then rerun doctor to inspect server connections.",
                );
            }
            Err(McpStatusFailure::Failed) => {
                return Finding::error(
                    &self.id,
                    "The already-open MCP runtime did not return server status.",
                    "Review the MCP runtime and rerun doctor.",
                );
            }
        };
        let Some(status) = statuses
            .iter()
            .find(|status| status.server_id == self.server_id)
        else {
            return Finding::error(
                &self.id,
                "The configured MCP server has no status in the open runtime.",
                "Review the MCP server configuration and reconnect it, then rerun doctor.",
            );
        };

        match status.status {
            tinymcp_bus::ServerStatus::Connected => {
                Finding::pass(&self.id, "The configured MCP server is connected.")
            }
            tinymcp_bus::ServerStatus::Disabled => Finding::skipped(
                &self.id,
                "The MCP server is disabled in the open runtime.",
                "Enable the MCP server to check its connection.",
            ),
            tinymcp_bus::ServerStatus::Connecting => Finding::new(
                &self.id,
                crate::doctor::FindingSeverity::Info,
                "The MCP server connection is still in progress.",
                "Wait for the connection attempt to finish, then rerun doctor.",
            ),
            tinymcp_bus::ServerStatus::Disconnected
            | tinymcp_bus::ServerStatus::Unauthorized
            | tinymcp_bus::ServerStatus::Error => Finding::error(
                &self.id,
                format!(
                    "The MCP server connection failed (status: {}).",
                    status.status.as_str()
                ),
                "Review the MCP server connection and authentication settings, then reconnect it.",
            ),
            _ => Finding::error(
                &self.id,
                "The MCP server reported an unsupported connection state.",
                "Review the MCP server status and rerun doctor.",
            ),
        }
    }
}

#[async_trait]
impl DoctorCheck for McpServerCheck {
    fn id(&self) -> &str {
        &self.id
    }

    fn category(&self) -> &str {
        "MCP servers"
    }

    fn tier(&self) -> DoctorTier {
        if self.duplicate_definitions {
            DoctorTier::Static
        } else {
            DoctorTier::Connect
        }
    }

    fn component_gate(&self) -> Option<ComponentId> {
        Some(ComponentId::Mcp)
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        vec![self.finding(ctx).await]
    }
}

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;
