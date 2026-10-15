//! MCP Registry — the host adapter for user-installed servers.
//!
//! TinyMCP owns the catalogs, store, connections, protocol, supervisor, OAuth,
//! and `mcp.json` document behavior. This module owns the OpenHuman RPC and
//! agent-tool surface, product admission policy, and event translation. A
//! server is declared in the user's `mcp.json`; there is no install-from-catalog
//! RPC.
//!
//! # Modules
//!
//! - [`bus`] — the lifecycle subscriber that logs this domain's events.
//! - [`ops`] — the `mcp_clients` RPC handlers, calling the module and
//!   publishing this application's own events.
//! - [`config_ops`] — the `config_get` / `config_set` handlers over the
//!   module-owned `mcp.json` document.
//! - `schemas` — the controller schemas and dispatch.
//! - [`supervisor_events`] — what the reconnect supervisor observed each
//!   tick, as this domain's events; the Event Log and the notification bridge
//!   read those (#5931).
//! - [`tools`] — the agent-facing tools.
//!
//! # The naming note still applies
//!
//! The RPC namespace and the database filename are `mcp_clients`, unchanged, so
//! existing frontend code and existing on-disk state keep working across the
//! move.
//!
//! # Types come from the contract
//!
//! Registry payloads are re-exported from `tinymcp_bus`. A parallel set of
//! types here would mean a conversion at every call site that nothing checks.

#[cfg(feature = "mcp")]
pub mod action_tool;
#[cfg(feature = "mcp")]
pub mod bus;
#[cfg(feature = "mcp")]
pub mod config_ops;
#[cfg(feature = "mcp")]
pub mod ops;
#[cfg(feature = "mcp")]
mod schemas;
#[cfg(feature = "mcp")]
pub mod supervisor_events;
#[cfg(feature = "mcp")]
pub mod tools;

#[cfg(feature = "mcp")]
pub use schemas::{
    all_controller_schemas as all_mcp_registry_controller_schemas,
    all_registered_controllers as all_mcp_registry_registered_controllers,
    schemas as mcp_registry_schemas,
};

/// The payload vocabulary, from the wire contract.
///
/// Re-exported under the path this module used to define them at, so callers
/// keep their spelling.
pub mod types {
    pub use tinymcp_bus::{
        CommandKind, ConnStatus, ConnectedServerOverview, InstalledServer, McpTool,
        RegistryConnection as SmitheryConnection, RegistryServerDetail as SmitheryServerDetail,
        RegistryServerSummary as SmitheryServerSummary, ServerStatus, Transport,
    };
}

pub use types::{ConnStatus, InstalledServer, McpTool};

/// Typed views over the module-owned live connection state.
///
/// These adapters query TinyMCP over the bus. Before the module is available,
/// read-only lookups report the empty state expected by callers that run before
/// boot completes.
#[cfg(feature = "mcp")]
pub mod connections {
    use crate::config::Config;
    pub use tinymcp_bus::ConnectedServerOverview;

    use crate::mcp::host;

    async fn module_call<R: serde::de::DeserializeOwned>(
        config: &Config,
        member: &str,
        args: impl serde::Serialize,
    ) -> Result<R, tinybus::module::ModuleCallError> {
        crate::mcp::module_client::McpConfiguredClient::for_config(config)?
            .call(&host::client_config(config), member, args)
            .await
    }

    /// Every connected server's identity and advertised tools.
    ///
    /// Sorted by qualified name, so a prompt built from this does not reshuffle
    /// between turns and cost its cached prefix.
    pub async fn connected_overview() -> Vec<ConnectedServerOverview> {
        let Some(config) = host::ambient_config() else {
            return Vec::new();
        };
        connected_overview_for_config(&config).await
    }

    /// Every connected server's identity and advertised tools in `config`'s
    /// workspace.
    ///
    /// The counterpart to [`connected_overview`] for a caller that holds a
    /// `Config`, for the same reason [`all_connected_tools_for_config`] and
    /// [`disconnect_for_config`] exist: the ambient form resolves through the
    /// process default, which stops answering once a second workspace is open.
    pub async fn connected_overview_for_config(config: &Config) -> Vec<ConnectedServerOverview> {
        match module_call(config, tinymcp_bus::names::methods::CONNECTED_OVERVIEW, ()).await {
            Ok(overview) => overview,
            Err(error) => {
                tracing::debug!(
                    ?error,
                    "[mcp] no MCP module for workspace; reporting no connections"
                );
                Vec::new()
            }
        }
    }

    /// Every enabled installed server's identity and tools in `config`'s
    /// workspace, without dialling: live tools for a connected server, the
    /// persistent tool cache for one that is not (yet).
    ///
    /// What the agent's MCP tool surface is built from, so a server's tools
    /// are offered from the first turn after a restart rather than only once
    /// its connect finishes. Listing is not authorization — a call still
    /// needs a live connection.
    pub async fn cached_overview_for_config(config: &Config) -> Vec<ConnectedServerOverview> {
        match module_call(config, tinymcp_bus::names::methods::CACHED_OVERVIEW, ()).await {
            Ok(overview) => overview,
            Err(error) => {
                tracing::debug!(
                    ?error,
                    "[mcp] no module cached tools; checking live connections"
                );
                connected_overview_for_config(config).await
            }
        }
    }

    /// Every tool on every connected server in `config`'s workspace.
    ///
    /// The counterpart to [`all_connected_tools`] for a caller that holds a
    /// `Config`. It resolves through [`host::for_config`], which is keyed by
    /// workspace, rather than through the process-wide default — so it answers
    /// about the workspace the caller named instead of whichever one
    /// `mcp::init` happened to claim first.
    ///
    /// That distinction is invisible in the shipped app, which opens one
    /// workspace, and decisive in a test binary: `resolve` hands back a lone
    /// host but returns `None` once a second one exists, so an ambient lookup
    /// silently reports nothing connected as soon as two tests each open their
    /// own temporary workspace in one process.
    ///
    /// A host that cannot be opened yields an empty list rather than an error:
    /// the callers fold this into a tool list, and MCP being unavailable must
    /// not fail the listing.
    pub async fn all_connected_tools_for_config(
        config: &Config,
    ) -> Vec<(String, String, tinymcp_bus::McpTool)> {
        connected_overview_for_config(config)
            .await
            .into_iter()
            .flat_map(|server| {
                let server_id = server.server_id;
                let qualified_name = server.qualified_name;
                server
                    .tools
                    .into_iter()
                    .map(move |tool| (server_id.clone(), qualified_name.clone(), tool))
            })
            .collect()
    }

    /// Every tool on every connected server, paired with its server.
    pub async fn all_connected_tools() -> Vec<(String, String, tinymcp_bus::McpTool)> {
        connected_overview()
            .await
            .into_iter()
            .flat_map(|server| {
                let server_id = server.server_id;
                let qualified_name = server.qualified_name;
                server
                    .tools
                    .into_iter()
                    .map(move |tool| (server_id.clone(), qualified_name.clone(), tool))
            })
            .collect()
    }

    /// The tools one connected server advertises, or `None` when it is not
    /// connected.
    pub async fn tools_for(server_id: &str) -> Option<Vec<tinymcp_bus::McpTool>> {
        let config = host::ambient_config()?;
        server_tools_for_config(&config, server_id).await
    }

    /// The tools one connected server advertises in `config`'s workspace.
    ///
    /// Named `server_tools_*` rather than `tools_for_config` so it cannot be
    /// misread as [`all_connected_tools_for_config`], which is the every-server
    /// form sitting a few lines above.
    ///
    /// `None` means "not connected". A workspace with no host at all logs and
    /// also yields `None`, because a server cannot be connected in a workspace
    /// that has no host — but the log is there so the two are distinguishable
    /// when this is the answer a caller did not expect.
    pub async fn server_tools_for_config(
        config: &Config,
        server_id: &str,
    ) -> Option<Vec<tinymcp_bus::McpTool>> {
        connected_overview_for_config(config)
            .await
            .into_iter()
            .find(|server| server.server_id == server_id)
            .map(|server| server.tools)
    }

    /// Whether a server has a live entry.
    pub async fn is_connected(server_id: &str) -> bool {
        let Some(config) = host::ambient_config() else {
            return false;
        };
        is_connected_for_config(&config, server_id).await
    }

    /// Whether a server has a live entry in `config`'s workspace.
    pub async fn is_connected_for_config(config: &Config, server_id: &str) -> bool {
        connected_overview_for_config(config)
            .await
            .iter()
            .any(|server| server.server_id == server_id)
    }

    /// Whether the last authentication failure requested OAuth.
    pub async fn auth_hint_for_config(config: &Config, server_id: &str) -> Option<&'static str> {
        let statuses: Result<Vec<tinymcp_bus::ConnStatus>, _> =
            module_call(config, tinymcp_bus::names::methods::STATUS, ()).await;
        match statuses {
            Ok(statuses) => statuses
                .iter()
                .find(|status| status.server_id == server_id)
                .and_then(|status| status.auth_hint.map(|hint| hint.as_code())),
            Err(error) => {
                tracing::debug!(?error, "[mcp] no status for workspace; no auth hint");
                None
            }
        }
    }

    /// Connects one server and returns the tools it advertised.
    ///
    /// # Errors
    ///
    /// Returns an error when the service is not up, or whatever the transport
    /// returns. A failed attempt is recorded either way, so a caller polling
    /// status sees the reason without re-attempting.
    pub async fn connect(
        config: &crate::config::Config,
        server: &tinymcp_bus::InstalledServer,
    ) -> anyhow::Result<Vec<tinymcp_bus::McpTool>> {
        let outcome: tinymcp_bus::ConnectOutcome = module_call(
            config,
            tinymcp_bus::names::methods::CONNECT,
            (server.server_id.clone(),),
        )
        .await
        .map_err(|error| anyhow::anyhow!("failed to connect `{}`: {error}", server.server_id))?;
        Ok(outcome.tools)
    }

    /// Drops a server's connection, reporting whether there was one.
    ///
    /// Answers `false` when the service is not up, which is the truth: nothing
    /// was holding a connection to drop.
    pub async fn disconnect(server_id: &str) -> bool {
        let Some(config) = host::ambient_config() else {
            return false;
        };
        disconnect_for_config(&config, server_id).await
    }

    /// Drop a connection held in `config`'s workspace.
    ///
    /// The counterpart to [`disconnect`] for a caller that holds a `Config`,
    /// for the same reason [`all_connected_tools_for_config`] exists: the
    /// by-server-id form resolves through the process default, which stops
    /// answering once a second workspace is open. A caller that connected
    /// through [`connect`] already named a workspace and should close over the
    /// same one.
    pub async fn disconnect_for_config(config: &Config, server_id: &str) -> bool {
        match module_call::<bool>(
            config,
            tinymcp_bus::names::methods::DISCONNECT,
            (server_id,),
        )
        .await
        {
            Ok(disconnected) => disconnected,
            Err(error) => {
                tracing::debug!(?error, "[mcp] no host for workspace; nothing to disconnect");
                false
            }
        }
    }

    /// The most recent failure message for a server in `config`'s workspace.
    pub async fn last_error_for_config(config: &Config, server_id: &str) -> Option<String> {
        match module_call::<Vec<tinymcp_bus::ConnStatus>>(
            config,
            tinymcp_bus::names::methods::STATUS,
            (),
        )
        .await
        {
            Ok(statuses) => statuses
                .into_iter()
                .find(|status| status.server_id == server_id)
                .and_then(|status| status.last_error),
            Err(error) => {
                tracing::debug!(
                    ?error,
                    server_id,
                    "[mcp] no host for workspace; no last error"
                );
                None
            }
        }
    }
}

/// Bringing installed servers up at startup.
#[cfg(feature = "mcp")]
pub mod boot {
    use crate::config::Config;
    use crate::mcp::{host, module_client::McpConfiguredClient};

    /// Connects every enabled installed server.
    ///
    /// Never fails: a server that cannot connect is logged and skipped, because
    /// one broken third-party integration must not stop the core coming up.
    pub async fn spawn_installed_servers(config: &Config) {
        let client = match McpConfiguredClient::for_config(config) {
            Ok(client) => client,
            Err(error) => {
                tracing::warn!("[mcp] the service could not be opened: {error}");
                return;
            }
        };

        // Opening the configured object starts TinyMCP's owned maintenance
        // task, whose boot pass connects installed servers. A static catalog
        // read forces that open without recreating connection policy here.
        if let Err(error) = client
            .call::<serde_json::Value>(
                &host::client_config(config),
                tinymcp_bus::names::methods::STATIC_LIST,
                (),
            )
            .await
        {
            tracing::warn!("[mcp] startup module open/connect pass failed: {error}");
        }
    }
}

/// Keeping installed servers connected.
#[cfg(feature = "mcp")]
pub mod supervisor {
    use std::time::Duration;

    use crate::mcp::{module_client::McpConfiguredClient, registry::supervisor_events};

    /// Runs the reconnect supervisor until the process ends.
    ///
    /// One task drains events from every open TinyMCP child. Probe scheduling,
    /// reconnection, and per-server backoff remain inside the owning module.
    /// The first poll is delayed a whole interval so it does not race startup.
    pub async fn run() {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            for (workspace, client) in McpConfiguredClient::open_clients() {
                match client.drain_supervisor_events(256).await {
                    Ok(Some(batch)) => {
                        supervisor_events::publish_batch(&workspace, &batch);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        tracing::debug!(?error, workspace = ?workspace, "[mcp] supervisor drain failed");
                    }
                }
            }
        }
    }
}

/// Browser sign-in, from the callback route's point of view.
#[cfg(feature = "mcp")]
pub mod oauth {
    use crate::config::Config;
    use crate::mcp::host;

    /// Finishes a sign-in from the redirect and reconnects the server.
    ///
    /// # Errors
    ///
    /// Returns a message when the state is unknown or expired, or when the
    /// token exchange fails.
    pub async fn complete(config: &Config, state: &str, code: &str) -> Result<String, String> {
        let outcome: tinymcp_bus::ConnectOutcome =
            crate::mcp::module_client::McpConfiguredClient::for_config(config)
                .map_err(|error| error.to_string())?
                .call(
                    &host::client_config(config),
                    tinymcp_bus::names::methods::OAUTH_COMPLETE,
                    (state, code),
                )
                .await
                .map_err(|error| error.to_string())?;

        Ok(outcome.server_id)
    }
}

/// Applies this application's prompt-injection policy to remote tool
/// definitions.
///
/// `tinymcp` returns definitions verbatim: the detector, its rules, and what a
/// hit means are this application's, and a module dropping tools by criteria of
/// its own would be making a decision it cannot explain to anyone.
///
/// A tool whose description trips a rule is dropped, and the drop is
/// logged and published with the *rule code* only — the offending text is never
/// re-emitted, because the payload is the thing that was dangerous.
#[cfg(feature = "mcp")]
pub(crate) fn tools_safe_for_agent(
    server: &str,
    tools: Vec<tinymcp_bus::McpTool>,
) -> Vec<tinymcp_bus::McpTool> {
    use crate::core::bus::BUS;
    use crate::core::events::DomainEvent;
    use crate::security::prompt_injection::scan_tool_definition;

    tools
        .into_iter()
        .filter(|tool| {
            let hit = tool
                .description
                .as_deref()
                .and_then(|text| scan_tool_definition("description", text));

            match hit {
                Some(hit) => {
                    tracing::warn!(
                        server,
                        tool = %tool.name,
                        reason = %hit.code,
                        "[mcp] dropped a remote tool that tripped the input-validation scan"
                    );
                    BUS.publish(DomainEvent::McpToolRejected {
                        server: server.to_string(),
                        tool: tool.name.clone(),
                        reason: hit.code.clone(),
                    });
                    false
                }
                None => true,
            }
        })
        .collect()
}

/// [`tools_safe_for_agent`] plus the security module's `ScanToolDefinition`.
///
/// The module verdict is context: a `Blocked` tool is dropped (with the rule ids
/// only, never the text), `Suspicious` and an unavailable module change nothing
/// beyond the host scan. It never re-admits a tool the host dropped.
#[cfg(feature = "mcp")]
pub(crate) async fn tools_safe_for_agent_scanned(
    server: &str,
    tools: Vec<tinymcp_bus::McpTool>,
) -> Vec<tinymcp_bus::McpTool> {
    let tools = tools_safe_for_agent(server, tools);
    #[cfg(feature = "security-module")]
    {
        use crate::modules::security::text::{scan_tool_definition, ScanVerdict};
        let Some(config) = crate::core::runtime::CoreContext::current_embedder_config() else {
            return tools;
        };
        if !config.modules.enabled {
            return tools;
        }
        let mut kept = Vec::with_capacity(tools.len());
        for tool in tools {
            let definition = serde_json::json!({
                "description": tool.description,
                "input_schema": tool.input_schema,
            });
            match scan_tool_definition(&config, &tool.name, &definition).await {
                Ok(scan) if scan.verdict == ScanVerdict::Blocked => {
                    tracing::warn!(
                        server,
                        tool = %tool.name,
                        rules = %scan.rule_ids.join(","),
                        "[mcp] dropped a remote tool blocked by the security module scan"
                    );
                }
                _ => kept.push(tool),
            }
        }
        // Needed when `security-module` is on: the fallback tail below is cfg-gated out.
        #[allow(clippy::needless_return)]
        return kept;
    }
    #[cfg(not(feature = "security-module"))]
    tools
}

// ---------------------------------------------------------------------------
// Disabled facade — compiled only when the `mcp` feature is OFF.
// ---------------------------------------------------------------------------

#[cfg(not(feature = "mcp"))]
mod stub;
#[cfg(not(feature = "mcp"))]
pub use stub::*;

#[cfg(all(test, feature = "mcp"))]
#[path = "mod_tests.rs"]
mod tests;
