//! MCP Registry — the host half of the user-installed server surface.
//!
//! The registry itself moved to `tinymcp`: the Smithery and official catalogs,
//! the SQLite store, the live connection map, the subprocess supervisor and the
//! browser sign-in flow all live there now. What is left here is what belongs
//! to *this* application. The catalogs are browse-only: a server is declared
//! in the user's `mcp.json` (`tinymcp::registry::config_doc`), never installed
//! from a listing.
//!
//! # Modules
//!
//! - [`bus`] — the lifecycle subscriber that logs this domain's events.
//! - [`store`] — the one direct reach into the registry's store that outlives
//!   the extraction: an end-to-end test seeding the upstream response cache.
//! - [`ops`] — the `mcp_clients` RPC handlers, delegating to the service
//!   [`super::host`] holds and publishing this application's own events.
//! - [`config_ops`] — the `config_get` / `config_set` handlers over the
//!   `mcp.json` document; the document contract and the reconciliation are
//!   `tinymcp::registry::config_doc`.
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
//! Everything this module used to define — the install record, the tool shape,
//! the status summary, the catalog records — is re-exported from
//! `tinymcp_bus`. A parallel set of types here would mean a conversion at every
//! call site that nothing checks.

#[cfg(feature = "mcp")]
pub mod action_tool;
#[cfg(feature = "mcp")]
pub mod bus;
#[cfg(feature = "mcp")]
pub mod config_ops;
#[cfg(feature = "mcp")]
pub(crate) mod helpers;
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

/// The live connection map.
///
/// A thin view over the service [`super::host`] holds. It exists because the
/// map used to be a process global and callers reached it through free
/// functions; `tinymcp` owns it instead, so those functions become lookups
/// through the holder. Everything here answers as though nothing were connected
/// when the service is not up yet, which is what a caller running before boot
/// completes should see.
#[cfg(feature = "mcp")]
pub mod connections {
    use crate::config::Config;
    pub use tinymcp_bus::ConnectedServerOverview;

    use crate::mcp::host;

    /// Every connected server's identity and advertised tools.
    ///
    /// Sorted by qualified name, so a prompt built from this does not reshuffle
    /// between turns and cost its cached prefix.
    pub async fn connected_overview() -> Vec<ConnectedServerOverview> {
        match host::try_service() {
            Some(service) => service.dynamic().connected_overview().await,
            None => Vec::new(),
        }
    }

    /// Every connected server's identity and advertised tools in `config`'s
    /// workspace.
    ///
    /// The counterpart to [`connected_overview`] for a caller that holds a
    /// `Config`, for the same reason [`all_connected_tools_for_config`] and
    /// [`disconnect_for_config`] exist: the ambient form resolves through the
    /// process default, which stops answering once a second workspace is open.
    pub async fn connected_overview_for_config(config: &Config) -> Vec<ConnectedServerOverview> {
        match host::for_config(config) {
            Ok(service) => service.dynamic().connected_overview().await,
            Err(error) => {
                tracing::debug!(
                    ?error,
                    "[mcp] no host for workspace; reporting no connections"
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
        let service = match host::for_config(config) {
            Ok(service) => service,
            Err(error) => {
                tracing::debug!(?error, "[mcp] no host for workspace; no cached tools");
                return Vec::new();
            }
        };
        match service.dynamic().cached_overview().await {
            Ok(overview) => overview,
            Err(error) => {
                tracing::debug!(%error, "[mcp] falling back to live servers only");
                service.dynamic().connected_overview().await
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
        match host::for_config(config) {
            Ok(service) => service.dynamic().connections().all_connected_tools().await,
            Err(error) => {
                tracing::debug!(?error, "[mcp] no host for workspace; reporting no tools");
                Vec::new()
            }
        }
    }

    /// Every tool on every connected server, paired with its server.
    pub async fn all_connected_tools() -> Vec<(String, String, tinymcp_bus::McpTool)> {
        match host::try_service() {
            Some(service) => service.dynamic().connections().all_connected_tools().await,
            None => Vec::new(),
        }
    }

    /// The tools one connected server advertises, or `None` when it is not
    /// connected.
    pub async fn tools_for(server_id: &str) -> Option<Vec<tinymcp_bus::McpTool>> {
        host::try_service()?
            .dynamic()
            .connections()
            .tools_for(server_id)
            .await
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
        match host::for_config(config) {
            Ok(service) => service.dynamic().connections().tools_for(server_id).await,
            Err(error) => {
                tracing::debug!(?error, server_id, "[mcp] no host for workspace; no tools");
                None
            }
        }
    }

    /// Whether a server has a live entry.
    pub async fn is_connected(server_id: &str) -> bool {
        match host::try_service() {
            Some(service) => {
                service
                    .dynamic()
                    .connections()
                    .is_connected(server_id)
                    .await
            }
            None => false,
        }
    }

    /// Whether a server has a live entry in `config`'s workspace.
    pub async fn is_connected_for_config(config: &Config, server_id: &str) -> bool {
        match host::for_config(config) {
            Ok(service) => {
                service
                    .dynamic()
                    .connections()
                    .is_connected(server_id)
                    .await
            }
            Err(error) => {
                tracing::debug!(
                    ?error,
                    server_id,
                    "[mcp] no host for workspace; reporting not connected"
                );
                false
            }
        }
    }

    /// Why a server's most recent attempt in `config`'s workspace hit a 401.
    pub async fn auth_hint_for_config(config: &Config, server_id: &str) -> Option<&'static str> {
        match host::for_config(config) {
            Ok(service) => Some(
                service
                    .dynamic()
                    .connections()
                    .auth_hint(server_id)
                    .await?
                    .as_code(),
            ),
            Err(error) => {
                tracing::debug!(
                    ?error,
                    server_id,
                    "[mcp] no host for workspace; no auth hint"
                );
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
        let service = host::for_config(config)?;
        let client = host::client_config(config);

        service
            .dynamic()
            .connections()
            .connect(
                service.dynamic().store(),
                service.dynamic().oauth(),
                &client.client_identity,
                client.proxy.as_ref(),
                server,
            )
            .await
            .map_err(|error| anyhow::anyhow!("failed to connect `{}`: {error}", server.server_id))
    }

    /// Drops a server's connection, reporting whether there was one.
    ///
    /// Answers `false` when the service is not up, which is the truth: nothing
    /// was holding a connection to drop.
    pub async fn disconnect(server_id: &str) -> bool {
        match host::try_service() {
            Some(service) => service.dynamic().connections().disconnect(server_id).await,
            None => false,
        }
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
        match host::for_config(config) {
            Ok(service) => service.dynamic().connections().disconnect(server_id).await,
            Err(error) => {
                tracing::debug!(?error, "[mcp] no host for workspace; nothing to disconnect");
                false
            }
        }
    }

    /// The most recent failure message for a server in `config`'s workspace.
    pub async fn last_error_for_config(config: &Config, server_id: &str) -> Option<String> {
        match host::for_config(config) {
            Ok(service) => service.dynamic().connections().last_error(server_id).await,
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
    use crate::mcp::host;

    /// Connects every enabled installed server.
    ///
    /// Never fails: a server that cannot connect is logged and skipped, because
    /// one broken third-party integration must not stop the core coming up.
    pub async fn spawn_installed_servers(config: &Config) {
        let service = match host::for_config(config) {
            Ok(service) => service,
            Err(error) => {
                tracing::warn!("[mcp] the service could not be opened: {error}");
                return;
            }
        };

        let client = host::client_config(config);
        let outcome = tinymcp::registry::connect_installed_servers(
            service.dynamic().store(),
            service.dynamic().connections(),
            service.dynamic().oauth(),
            &client.client_identity,
            client.proxy.as_ref(),
        )
        .await;

        tracing::info!(
            connected = outcome.connected,
            failed = outcome.failed,
            skipped = outcome.skipped,
            "[mcp] startup connect finished"
        );
        crate::mcp::ui::discovery::log_all_installed(service).await;
    }
}

/// Keeping installed servers connected.
#[cfg(feature = "mcp")]
pub mod supervisor {
    use crate::mcp::host;

    /// Runs the reconnect supervisor until the process ends.
    ///
    /// One task for every host the process has opened, driven by
    /// `tinymcp::Supervisor::run_many`: the connection map is per-workspace,
    /// and a host opened after boot — a workspace switch — is supervised from
    /// the tick after it appears, with its backoff state kept per workspace.
    /// The first tick is delayed a whole interval so it does not race the
    /// startup connect pass.
    pub async fn run() {
        tinymcp::Supervisor::run_many(
            tinymcp::SupervisorConfig::default(),
            // Every host currently open. The identity and proxy each was
            // opened with ride along, so a reconnect dials the way the host's
            // own connections do.
            || {
                host::all_hosts()
                    .into_iter()
                    .map(
                        |(workspace, registry, identity, proxy)| tinymcp::SupervisedHost {
                            key: workspace,
                            registry,
                            identity,
                            proxy,
                        },
                    )
                    .collect()
            },
            // What the tick observed becomes this domain's events, so a probe
            // outcome reaches the Event Log and a server that stays down
            // reaches the user (#5931). The workspace goes with them: a
            // subscriber that persists or announces one must not take a
            // switched-away workspace's outage for its own.
            |workspace, report| {
                super::supervisor_events::publish(workspace, report);
            },
        )
        .await;
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
        let outcome = host::for_config(config)
            .map_err(|error| error.to_string())?
            .dynamic()
            .oauth_complete(state, code)
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

// ---------------------------------------------------------------------------
// Disabled facade — compiled only when the `mcp` feature is OFF.
// ---------------------------------------------------------------------------

#[cfg(not(feature = "mcp"))]
mod stub;
#[cfg(not(feature = "mcp"))]
pub use stub::*;
