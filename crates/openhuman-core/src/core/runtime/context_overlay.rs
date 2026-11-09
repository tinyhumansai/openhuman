//! The per-agent overrides a derived [`CoreContext`](super::CoreContext)
//! is built from.

use std::sync::Arc;

/// Where a derived context gets its autonomy tier for live host gates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AutonomyTierSource {
    /// Reload the context's anchored config file for each gated operation.
    #[default]
    LiveConfig,
    /// Keep the effective tier supplied by an in-memory agent configuration.
    Captured,
}

/// Per-agent overrides layered onto a booted context by
/// [`CoreContext::derive_with`](super::CoreContext::derive_with).
///
/// This is the seam a library host uses to run many independently configured
/// agents on one booted core: each agent gets its own `Config` (provider
/// routes, MCP servers, autonomy tier, `action_dir`), its own
/// [`DomainSet`](crate::core::runtime::DomainSet), its own
/// [`ToolGroups`](crate::tools::toolpacks::ToolGroups) and its own skill-root
/// policy, while sharing the host identity, keyring, bus and RPC bearer of the
/// context it derives from.
#[derive(Debug, Clone)]
pub struct ContextOverlay {
    /// Host adapters local to this agent; absent inherits the parent.
    pub host_overrides: Option<Arc<crate::agent::host_overrides::HostOverrides>>,
    /// The config every handler dispatched under the derived context reads
    /// through `config::ops::load_config_with_timeout()`. Keep `config_path`
    /// equal to the parent's: credentials, auth profiles and the keyring file
    /// backend all resolve against its parent directory.
    pub config: crate::config::Config,
    /// Domain families live for the derived context. Only narrowing the parent
    /// is meaningful: controllers a booted core never registered stay absent
    /// no matter what this says.
    pub domains: crate::core::runtime::DomainSet,
    /// Tool-group disclosure for the derived context.
    pub tool_groups: crate::tools::toolpacks::ToolGroups,
    /// Scan the operator's user-scope skill roots (`true` = today's behaviour).
    pub user_skill_roots: bool,
    /// The agent a host session store scopes this context's transcripts,
    /// journal, goals and todos to. `None` keeps the parent's. Setting it
    /// gives the derived context state slots of its own.
    pub session_agent: Option<String>,
    /// The tenant (SaaS profile) the derived context serves. `None` keeps the
    /// parent's. Setting it gives the derived context state slots of its own
    /// and keys every tenant-scoped table on it
    /// ([`tenant`](crate::core::runtime::tenant)).
    pub profile: Option<String>,
    /// The agent's own security policy: autonomy tier, auto-approve list,
    /// action budget. `None` keeps the parent's.
    pub agent_policy: Option<Arc<crate::security::SecurityPolicy>>,
    /// Source binding for autonomy-tier gates under this context.
    pub autonomy_tier_source: Option<AutonomyTierSource>,
    /// A fixed maximum autonomy level inherited by derived contexts.
    pub autonomy_tier_ceiling: Option<crate::security::AutonomyLevel>,
    /// Turn the interactive approval gate off for this agent.
    pub approvals_disabled: bool,
    /// The agent's own sub-agent catalogue. `None` keeps the parent's.
    pub definitions: Option<Arc<crate::agent_registry::host_definition::AgentDefinitionRegistry>>,
}

impl ContextOverlay {
    /// An overlay that keeps user-scope skill roots visible.
    pub fn new(
        config: crate::config::Config,
        domains: crate::core::runtime::DomainSet,
        tool_groups: crate::tools::toolpacks::ToolGroups,
    ) -> Self {
        Self {
            host_overrides: None,
            config,
            domains,
            tool_groups,
            user_skill_roots: true,
            session_agent: None,
            profile: None,
            agent_policy: None,
            autonomy_tier_source: None,
            autonomy_tier_ceiling: None,
            approvals_disabled: false,
            definitions: None,
        }
    }

    /// Bind agent-local hooks, native inference and session storage.
    pub fn host_overrides(
        mut self,
        overrides: Arc<crate::agent::host_overrides::HostOverrides>,
    ) -> Self {
        self.host_overrides = Some(overrides);
        self
    }

    /// Hide the operator's `~/.openhuman/skills` / `~/.agents/skills` from
    /// skill discovery under the derived context.
    pub fn without_user_skill_roots(mut self) -> Self {
        self.user_skill_roots = false;
        self
    }

    /// Scope a host session store to `agent_id` under the derived context.
    pub fn session_agent(mut self, agent_id: impl Into<String>) -> Self {
        self.session_agent = Some(agent_id.into());
        self
    }

    /// Make the derived context serve tenant (profile) `profile_id`.
    pub fn profile(mut self, profile_id: impl Into<String>) -> Self {
        self.profile = Some(profile_id.into());
        self
    }

    /// Give the derived context its own security policy.
    pub fn agent_policy(mut self, policy: Arc<crate::security::SecurityPolicy>) -> Self {
        self.agent_policy = Some(policy);
        self
    }

    /// Keep autonomy tier bound to this context's effective in-memory config.
    pub fn captured_autonomy_tier(mut self) -> Self {
        self.autonomy_tier_source = Some(AutonomyTierSource::Captured);
        self
    }

    /// Keep this context's effective autonomy tier at ReadOnly.
    pub fn read_only_autonomy_ceiling(mut self) -> Self {
        self.autonomy_tier_ceiling = Some(crate::security::AutonomyLevel::ReadOnly);
        self
    }

    /// Give the derived context its own sub-agent catalogue.
    pub fn definitions(
        mut self,
        definitions: Arc<crate::agent_registry::host_definition::AgentDefinitionRegistry>,
    ) -> Self {
        self.definitions = Some(definitions);
        self
    }
}

pub(super) fn stricter_autonomy_ceiling(
    left: Option<crate::security::AutonomyLevel>,
    right: Option<crate::security::AutonomyLevel>,
) -> Option<crate::security::AutonomyLevel> {
    use crate::security::AutonomyLevel::{Full, ReadOnly, Supervised};

    match (left, right) {
        (None, ceiling) | (ceiling, None) => ceiling,
        (Some(ReadOnly), _) | (_, Some(ReadOnly)) => Some(ReadOnly),
        (Some(Supervised), _) | (_, Some(Supervised)) => Some(Supervised),
        (Some(Full), Some(Full)) => Some(Full),
    }
}
