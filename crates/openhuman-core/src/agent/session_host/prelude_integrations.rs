//! Turn-boundary refresh of the prelude's integration state: hydrating the
//! connected integrations a session starts without, tracking connects and
//! revokes between turns, and adopting the integration actions the tinyagents
//! session restored for a resumed thread.

use crate::core::runtime::DispatchCtx;
use std::sync::Arc;

use tinyagents_runtime::ToolSnapshot;

use super::OpenHumanTurnPrelude;

/// Module-projected catalogs and the names frozen in the resumed transcript.
#[cfg(feature = "mcp")]
#[derive(Default)]
pub(super) struct McpToolCatalog {
    pub(super) connected: Vec<tinymcp_bus::McpToolPresentation>,
    pub(super) configured: Vec<tinymcp_bus::McpToolPresentation>,
    pub(super) recorded_names: std::collections::HashSet<String>,
    pub(super) recorded_specs: Vec<tinytools::ToolSpec>,
}

impl OpenHumanTurnPrelude {
    /// The session's own definition, else the catalogue's entry for its id.
    pub(super) fn session_definition(
        &self,
        registry: &crate::agent::harness::host_definition::AgentDefinitionRegistry,
    ) -> Option<crate::agent::harness::host_definition::AgentDefinition> {
        let definition = self
            .definition
            .as_deref()
            .cloned()
            .or_else(|| registry.get(&self.agent_definition_id).cloned());
        if definition.is_none() {
            tracing::trace!(
                agent = %self.agent_definition_id,
                "[session] no definition resolved; delegation surface unchanged"
            );
        }
        definition
    }

    /// Takes the declarations the tinyagents session restored for this
    /// thread. Called before the boundary refresh so the rebuilt surface can
    /// include them.
    pub(super) fn adopt_recorded_tools(&self, recorded: Option<&ToolSnapshot>) {
        let Some(recorded) = recorded else {
            return;
        };
        let actions = super::super::recorded_tools::recorded_integration_actions(recorded.specs());
        log::debug!(
            "[session] adopting {} recorded integration action declaration(s) agent={}",
            actions.len(),
            self.agent_definition_id
        );
        // Search role tools ride in the same list; each rehydration pass
        // keeps only the names it owns.
        let mut actions = actions;
        actions.extend(super::super::recorded_tools::recorded_search_tools(
            recorded.specs(),
        ));
        let mut mutable = self
            .mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        mutable.recorded_integration_actions = actions;
        mutable.recorded_tool_snapshot_adopted = true;
        {
            mutable.recorded_repl_tools = recorded
                .specs()
                .iter()
                .filter(|spec| crate::inference::tokenjuice::is_repl_tool(&spec.name))
                .cloned()
                .collect();
        }
        // MCP tools are rebuilt from the tool cache every turn; all a resumed
        // thread needs remembered is which names it was sent, so a tool it
        // knew under the pre-readable hashed name keeps resolving.
        #[cfg(feature = "mcp")]
        {
            let recorded_mcp_specs: Vec<tinytools::ToolSpec> = recorded
                .specs()
                .iter()
                .filter(|spec| spec.name.starts_with("mcp_"))
                .cloned()
                .collect();
            mutable.mcp_tools.recorded_names = recorded_mcp_specs
                .iter()
                .map(|spec| spec.name.clone())
                .collect();
            mutable.mcp_tools.recorded_specs = recorded_mcp_specs;
        }
    }

    /// Executors for tool declarations the resumed thread was sent that the
    /// live surface did not supply this turn.
    ///
    /// * Integration actions stay executable even when this process has not
    ///   (re)fetched their integration yet — only for an agent that carries
    ///   integration actions at all.
    /// * Search role tools stay declared even when no provider is usable now
    ///   (signed out, provider turned off); a call answers with an actionable
    ///   error instead of an unknown-tool failure.
    pub(super) fn rebuilt_recorded_tools(
        &self,
        definition: &crate::agent::harness::host_definition::AgentDefinition,
        base: &[Box<dyn tinytools::Tool>],
        synthesized: &[Box<dyn tinytools::Tool>],
        integrations: &[crate::agent::prompts::ConnectedIntegration],
        integrations_are_authoritative: bool,
    ) -> Vec<Box<dyn tinytools::Tool>> {
        let recorded = self
            .mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .recorded_integration_actions
            .clone();
        let mut rebuilt = Vec::new();
        if definition.subagents.iter().any(|entry| {
            matches!(
                entry,
                tinyagents_definition::SubagentEntry::Skills(wildcard)
                    if wildcard.matches_all()
            )
        }) {
            rebuilt = super::super::recorded_tools::rehydrate_integration_actions(
                &recorded,
                synthesized,
                integrations,
                integrations_are_authoritative,
            );
            if !rebuilt.is_empty() {
                log::info!(
                    "[session] rebuilt {} recorded integration action(s) the live integrations did not supply agent={}",
                    rebuilt.len(),
                    self.agent_definition_id
                );
            }
        }
        #[cfg(feature = "modules")]
        rebuilt.extend(super::super::recorded_tools::rehydrate_search_tools(
            &recorded,
            &[base, synthesized],
            &self.agent_definition_id,
        ));
        #[cfg(not(feature = "modules"))]
        let _ = base;
        {
            let recorded_repl_tools = self
                .mutable
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .recorded_repl_tools
                .clone();
            let live_names = base
                .iter()
                .chain(synthesized.iter())
                .map(|tool| tool.name())
                .collect::<std::collections::HashSet<_>>();
            let missing = recorded_repl_tools
                .into_iter()
                .filter(|spec| !live_names.contains(spec.name.as_str()))
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                rebuilt.extend(
                    crate::inference::tokenjuice::repl_tools::repl_tools_for_recorded(
                        self.runtime_config.as_deref(),
                        &self.workspace_dir,
                        &missing,
                    ),
                );
            }
        }
        rebuilt
    }

    pub(super) async fn refresh_turn_boundary(&self, cold: bool) -> anyhow::Result<()> {
        // Hydrate on the first turn of *this session instance*, not only on a
        // brand-new thread. A resumed thread is never `cold`, and a session
        // rebuilt after a restart is seeded from an empty cache — gating the fetch on
        // `cold` left it with zero integrations, no deferred Composio
        // actions, and no `tool_search` bridge for the whole thread.
        // `refresh_cold_integrations` is a no-op once hydrated.
        self.refresh_cold_integrations().await;
        #[cfg(feature = "mcp")]
        self.refresh_connected_mcp_tools().await;
        #[cfg(feature = "mcp")]
        self.refresh_configured_mcp_tools().await;
        if !cold {
            self.refresh_dynamic_announcements().await;
        }
        // Integration changes are authority changes, not only display
        // announcements. Refresh the delegation executable set and rebuild
        // its schema/policy in the same hook pass before the driver sees it.
        self.refresh_delegation_tool_surface()?;
        Ok(())
    }

    /// Snapshot the installed servers' tools for this workspace, from the
    /// persistent tool cache when a server is not connected yet. A disabled or
    /// uninstalled server drops out of the next turn's search catalogue; a
    /// merely disconnected one stays listed and its calls report that it is
    /// not connected.
    #[cfg(feature = "mcp")]
    async fn refresh_connected_mcp_tools(&self) {
        let config = match self.runtime_config.as_deref() {
            Some(config) => config,
            None => {
                self.mutable
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .mcp_tools
                    .connected
                    .clear();
                return;
            }
        };
        let servers = crate::mcp::registry::connections::cached_overview_for_config(config).await;
        let sources = servers
            .into_iter()
            .map(|server| tinymcp_bus::McpToolSourceSpec {
                server_id: server.server_id.clone(),
                label: server.qualified_name.clone(),
                family: server.qualified_name,
                display_name: server.display_name,
                tools: crate::mcp::registry::tools_safe_for_agent(&server.server_id, server.tools),
                default_exposure: tinymcp_bus::McpToolExposure::Deferred,
                direct_tools: Vec::new(),
            })
            .collect::<Vec<_>>();
        let prepared = match crate::mcp::module_client::McpConfiguredClient::for_config(config) {
            Ok(client) => client
                .call(
                    &crate::mcp::host::client_config(config),
                    tinymcp_bus::names::methods::PREPARE_AGENT_TOOLS,
                    (tinymcp_bus::PrepareAgentToolsRequest { sources },),
                )
                .await
                .unwrap_or_else(|error| {
                    tracing::debug!(?error, "[mcp] module tool presentation unavailable");
                    Vec::new()
                }),
            Err(error) => {
                tracing::debug!(?error, "[mcp] no module for tool presentation");
                Vec::new()
            }
        };
        let count = prepared.len();
        tracing::debug!(
            agent = %self.agent_definition_id,
            tools = count,
            "[mcp] refreshed deferred tool catalogue"
        );
        self.mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .mcp_tools
            .connected = prepared;
    }

    /// Refreshes configured static-server declarations from TinyMCP's
    /// persistent owner cache, listing only when a server has no cached
    /// catalog yet.
    #[cfg(feature = "mcp")]
    async fn refresh_configured_mcp_tools(&self) {
        let Some(config) = self.runtime_config.as_deref() else {
            self.mutable
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .mcp_tools
                .configured
                .clear();
            return;
        };
        let presentations =
            match crate::mcp::module_client::McpConfiguredClient::configured_tool_presentations(
                config,
            )
            .await
            {
                Ok(presentations) => presentations,
                Err(error) => {
                    tracing::debug!(?error, "[mcp] configured tool catalog is unavailable");
                    Vec::new()
                }
            };
        self.mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .mcp_tools
            .configured = presentations;
    }

    /// Construct searchable MCP actions from the workspace's current snapshot.
    /// Called before the tool-surface lock is taken to keep lock order stable.
    #[cfg(feature = "mcp")]
    pub(super) fn collect_mcp_search_tools(&self) -> Vec<Box<dyn tinytools::Tool>> {
        if !self
            .definition
            .as_ref()
            .is_some_and(|definition| definition.extensions.searches_connected_mcp)
        {
            tracing::trace!(
                agent = %self.agent_definition_id,
                "[mcp] agent does not search connected MCP tools"
            );
            return Vec::new();
        }
        let Some(config) = self.runtime_config.as_ref() else {
            return Vec::new();
        };
        let (servers, recorded) = {
            let mutable = self
                .mutable
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (
                mutable.mcp_tools.connected.clone(),
                mutable.mcp_tools.recorded_names.clone(),
            )
        };
        crate::mcp::registry::action_tool::deferred_connected_tools_with_legacy(
            Arc::clone(config),
            &servers,
            &recorded,
        )
    }

    /// Builds configured static MCP executors from the module-prepared cache.
    #[cfg(feature = "mcp")]
    pub(super) fn collect_configured_mcp_tools(&self) -> Vec<Box<dyn tinytools::Tool>> {
        let Some(config) = self.runtime_config.as_deref() else {
            return Vec::new();
        };
        let presentations = self
            .mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .mcp_tools
            .configured
            .clone();
        let security = Arc::new(crate::security::SecurityPolicy::from_full_config(config));
        crate::tools::implementations::network::configured_server_tools_from_presentations(
            config,
            presentations,
            &security,
        )
    }

    pub(super) async fn refresh_cold_integrations(&self) {
        let should_fetch = !self
            .mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .connected_integrations_initialized;
        if !should_fetch {
            return;
        }
        let config = match self.runtime_config.clone() {
            Some(config) => Some(config),
            None => crate::config::ops::load_current_or_init(&self.dispatch_ctx())
                .await
                .ok()
                .map(Arc::new),
        };
        let Some(config) = config else {
            return;
        };
        let Some((connected, authoritative)) = load_connected_integrations(&config).await else {
            // Backend unreachable and nothing cached: stay un-hydrated so the
            // next turn retries rather than pinning an empty surface.
            log::warn!(
                "[session] integrations unavailable and no cached snapshot; will retry next turn agent={}",
                self.agent_definition_id
            );
            return;
        };
        log::info!(
            "[session] hydrated connected integrations count={} agent={}",
            connected.len(),
            self.agent_definition_id
        );
        let mcp_servers = crate::mcp::registry::connections::connected_overview()
            .await
            .into_iter()
            .map(|server| server.qualified_name)
            .collect::<std::collections::HashSet<_>>();
        let mut mutable = self
            .mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        apply_cold_hydration(&mut mutable, connected, authoritative, mcp_servers);
    }

    pub(super) async fn refresh_dynamic_announcements(&self) {
        let skills_changed = self.drain_host_events();
        let config = match self.runtime_config.clone() {
            Some(config) => Some(config),
            None => crate::config::ops::load_current_or_init(&self.dispatch_ctx())
                .await
                .ok()
                .map(Arc::new),
        };
        if let Some(config) = config.as_deref() {
            // The connection list and change events invalidate the process
            // snapshot, so the turn reads it without an idle-time refresh.
            let current = match crate::integrations::composio::cached_active_integrations(config) {
                Some(current) => Some((current, true)),
                None => load_connected_integrations(config).await,
            };
            if let Some((current, authoritative)) = current {
                let mut mutable = self
                    .mutable
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let state = &mut *mutable;
                merge_integration_announcements(
                    &mut state.announced_integrations,
                    &mut state.pending_integration_announcement,
                    &current,
                );
                mutable.connected_integrations = current;
                mutable.connected_integrations_authoritative = authoritative;
            }
        }
        let connected_mcp = crate::mcp::registry::connections::connected_overview()
            .await
            .into_iter()
            .map(|server| server.qualified_name)
            .collect::<Vec<_>>();
        let mut mutable = self
            .mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let connected_mcp: std::collections::HashSet<_> = connected_mcp.into_iter().collect();
        mutable
            .announced_mcp_servers
            .retain(|server| connected_mcp.contains(server));
        mutable
            .pending_mcp_announcement
            .retain(|server| connected_mcp.contains(server));
        for server in connected_mcp {
            if mutable.announced_mcp_servers.insert(server.clone())
                && !mutable.pending_mcp_announcement.contains(&server)
            {
                mutable.pending_mcp_announcement.push(server);
            }
        }
        if !skills_changed {
            return;
        }
        // Event-driven metadata refresh keeps the steady-state hot path free
        // of the old per-turn filesystem scan.
        let latest = crate::skills::load_workflow_metadata(&self.workspace_dir);
        let id = |workflow: &tinyskills::Skill| {
            if workflow.dir_name.is_empty() {
                workflow.name.clone()
            } else {
                workflow.dir_name.clone()
            }
        };
        let previous: std::collections::HashSet<_> = mutable.workflows.iter().map(&id).collect();
        let current: std::collections::HashSet<_> = latest.iter().map(&id).collect();
        for id in current.difference(&previous) {
            if mutable.announced_skills.insert((*id).clone())
                && !mutable.pending_skill_announcement.contains(id)
            {
                mutable.pending_skill_announcement.push((*id).clone());
            }
        }
        for id in previous.difference(&current) {
            mutable.announced_skills.remove(id);
            mutable
                .pending_skill_announcement
                .retain(|pending| pending != id);
            if !mutable.pending_skill_retraction.contains(id) {
                mutable.pending_skill_retraction.push((*id).clone());
            }
        }
        mutable.workflows = latest;
    }
}

/// Live connected integrations, falling back to the last cached snapshot
/// when the backend is unreachable. `None` only when
/// there is neither a live answer nor any snapshot to fall back to.
async fn load_connected_integrations(
    config: &crate::config::Config,
) -> Option<(Vec<crate::agent::prompts::ConnectedIntegration>, bool)> {
    use crate::integrations::composio::FetchConnectedIntegrationsStatus;
    match crate::integrations::composio::fetch_connected_integrations_status(config).await {
        FetchConnectedIntegrationsStatus::Authoritative(connected) => Some((connected, true)),
        FetchConnectedIntegrationsStatus::Unavailable => {
            let stale = crate::integrations::composio::cached_active_integrations(config);
            log::warn!(
                "[session] integrations fetch unavailable; using stale snapshot={}",
                stale.as_ref().map_or(0, Vec::len)
            );
            stale.map(|connected| (connected, false))
        }
    }
}

/// Store a cold-hydration result on the prelude state.
///
/// The announced sets are seeded only on the first hydration of this session
/// instance. A stale fallback leaves hydration pending, so a later turn
/// hydrates again; reseeding then would mark a toolkit connected in between
/// as already announced and the model would never hear of it. Later
/// hydrations therefore diff against the existing sets instead.
pub(super) fn apply_cold_hydration(
    state: &mut super::OpenHumanTurnPreludeMutable,
    connected: Vec<crate::agent::prompts::ConnectedIntegration>,
    authoritative: bool,
    mcp_servers: std::collections::HashSet<String>,
) {
    // A stale fallback is useful for announcements but cannot authorize
    // restored executors. Leave hydration pending so a later turn retries
    // the live lookup rather than pinning this session to the snapshot.
    state.connected_integrations_initialized = authoritative;
    state.connected_integrations_authoritative = authoritative;
    if state.integration_announcements_seeded {
        log::debug!(
            "[session] re-hydrating integrations after a stale snapshot; diffing announcements"
        );
        merge_integration_announcements(
            &mut state.announced_integrations,
            &mut state.pending_integration_announcement,
            &connected,
        );
        for server in &mcp_servers {
            if state.announced_mcp_servers.insert(server.clone())
                && !state.pending_mcp_announcement.contains(server)
            {
                state.pending_mcp_announcement.push(server.clone());
            }
        }
        state
            .announced_mcp_servers
            .retain(|server| mcp_servers.contains(server));
        state
            .pending_mcp_announcement
            .retain(|server| mcp_servers.contains(server));
    } else {
        // Seed only the toolkits the user actually connected: the list also
        // carries every allowlisted-but-unconnected toolkit (`connected:
        // false`), and seeding those made a later diff announce them as
        // "connected" (feedback: "100+ services connected").
        state.announced_integrations = connected_toolkit_slugs(&connected);
        state.announced_mcp_servers = mcp_servers;
        state.integration_announcements_seeded = true;
    }
    state.connected_integrations = connected;
}

/// Toolkit slugs the user has an active connection for. The integration list
/// also carries every allowlisted toolkit with `connected: false`; those are
/// available to connect, not connected, and must never be announced as such.
fn connected_toolkit_slugs(
    items: &[crate::agent::prompts::ConnectedIntegration],
) -> std::collections::HashSet<String> {
    items
        .iter()
        .filter(|item| item.connected)
        .map(|item| item.toolkit.clone())
        .collect()
}

/// Diff the live integration list against what this session already
/// announced: drop announcements for toolkits that are no longer connected
/// and queue an `[integration update]` for each newly connected one. Only
/// `connected` items count (see [`connected_toolkit_slugs`]).
fn merge_integration_announcements(
    announced: &mut std::collections::HashSet<String>,
    pending: &mut Vec<String>,
    current: &[crate::agent::prompts::ConnectedIntegration],
) {
    let current_slugs = connected_toolkit_slugs(current);
    announced.retain(|slug| current_slugs.contains(slug));
    pending.retain(|slug| current_slugs.contains(slug));
    let mut added: Vec<&String> = current_slugs
        .iter()
        .filter(|slug| !announced.contains(*slug))
        .collect();
    added.sort();
    for slug in added {
        announced.insert(slug.clone());
        if !pending.contains(slug) {
            pending.push(slug.clone());
        }
    }
    tracing::debug!(
        connected = current_slugs.len(),
        listed = current.len(),
        pending = pending.len(),
        "[session] integration announcements diffed against connected toolkits"
    );
}

#[cfg(test)]
#[path = "prelude_integrations_tests.rs"]
mod tests;
