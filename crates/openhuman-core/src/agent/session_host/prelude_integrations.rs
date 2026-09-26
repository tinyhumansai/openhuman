//! Turn-boundary refresh of the prelude's integration state: hydrating the
//! connected integrations a session starts without, tracking connects and
//! revokes between turns, and adopting the integration actions the tinyagents
//! session restored for a resumed thread.

use std::sync::Arc;

use tinyagents_runtime::ToolSnapshot;

use super::OpenHumanTurnPrelude;

impl OpenHumanTurnPrelude {
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
        self.mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .recorded_integration_actions = actions;
    }

    pub(super) async fn refresh_turn_boundary(&self, cold: bool) {
        // Hydrate on the first turn of *this session instance*, not only on a
        // brand-new thread. A resumed thread is never `cold`, and a session
        // rebuilt after a restart is seeded from an empty cache — gating the fetch on
        // `cold` left it with zero integrations, no deferred Composio
        // actions, and no `tool_search` bridge for the whole thread.
        // `refresh_cold_integrations` is a no-op once hydrated.
        self.refresh_cold_integrations().await;
        #[cfg(feature = "mcp")]
        self.refresh_connected_mcp_tools().await;
        if !cold {
            self.refresh_dynamic_announcements().await;
        }
        // Integration changes are authority changes, not only display
        // announcements. Refresh the delegation executable set and rebuild
        // its schema/policy in the same hook pass before the driver sees it.
        self.refresh_delegation_tool_surface();
    }

    /// Snapshot the currently connected server actions for this workspace.
    /// A disconnected server drops out of the next turn's search catalogue.
    #[cfg(feature = "mcp")]
    async fn refresh_connected_mcp_tools(&self) {
        let servers = match self.runtime_config.as_deref() {
            Some(config) => {
                crate::mcp::registry::connections::connected_overview_for_config(config).await
            }
            None => Vec::new(),
        };
        let count = servers
            .iter()
            .map(|server| server.tools.len())
            .sum::<usize>();
        tracing::debug!(
            agent = %self.agent_definition_id,
            servers = servers.len(),
            tools = count,
            "[mcp] refreshed deferred tool catalogue"
        );
        self.mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .connected_mcp_tools = servers;
    }

    /// Construct searchable MCP actions from the workspace's current snapshot.
    /// Called before the tool-surface lock is taken to keep lock order stable.
    #[cfg(feature = "mcp")]
    pub(super) fn collect_mcp_search_tools(&self) -> Vec<Box<dyn tinytools::Tool>> {
        if self.agent_definition_id != "orchestrator" {
            return Vec::new();
        }
        let Some(config) = self.runtime_config.as_ref() else {
            return Vec::new();
        };
        let servers = self
            .mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .connected_mcp_tools
            .clone();
        crate::mcp::registry::action_tool::deferred_connected_tools(Arc::clone(config), &servers)
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
            None => crate::config::Config::load_or_init()
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
        mutable.connected_integrations = connected;
        // A stale fallback is useful for announcements but cannot authorize
        // restored executors. Leave hydration pending so a later turn retries
        // the live lookup rather than pinning this session to the snapshot.
        mutable.connected_integrations_initialized = authoritative;
        mutable.connected_integrations_authoritative = authoritative;
        mutable.announced_integrations = mutable
            .connected_integrations
            .iter()
            .map(|item| item.toolkit.clone())
            .collect();
        mutable.announced_mcp_servers = mcp_servers;
    }

    pub(super) async fn refresh_dynamic_announcements(&self) {
        let skills_changed = self.drain_host_events();
        let config = match self.runtime_config.clone() {
            Some(config) => Some(config),
            None => crate::config::Config::load_or_init()
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
                let current_slugs: std::collections::HashSet<_> =
                    current.iter().map(|item| item.toolkit.clone()).collect();
                mutable
                    .announced_integrations
                    .retain(|slug| current_slugs.contains(slug));
                mutable
                    .pending_integration_announcement
                    .retain(|slug| current_slugs.contains(slug));
                for slug in &current_slugs {
                    if mutable.announced_integrations.insert(slug.clone())
                        && !mutable.pending_integration_announcement.contains(slug)
                    {
                        mutable.pending_integration_announcement.push(slug.clone());
                    }
                }
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
        let id = |workflow: &crate::skills::Workflow| {
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
/// (even past its TTL) when the backend is unreachable. `None` only when
/// there is neither a live answer nor any snapshot to fall back to.
async fn load_connected_integrations(
    config: &crate::config::Config,
) -> Option<(Vec<crate::agent::prompts::ConnectedIntegration>, bool)> {
    use crate::integrations::composio::FetchConnectedIntegrationsStatus;
    match crate::integrations::composio::fetch_connected_integrations_status(config).await {
        FetchConnectedIntegrationsStatus::Authoritative(connected) => Some((connected, true)),
        FetchConnectedIntegrationsStatus::Unavailable => {
            let stale =
                crate::integrations::composio::cached_active_integrations_including_expired(config);
            log::warn!(
                "[session] integrations fetch unavailable; using stale snapshot={}",
                stale.as_ref().map_or(0, Vec::len)
            );
            stale.map(|connected| (connected, false))
        }
    }
}
