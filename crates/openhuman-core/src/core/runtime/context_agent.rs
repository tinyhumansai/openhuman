//! What a context derived for one agent owns: its policy, approval switch,
//! sub-agent catalogue and state slots.

use super::*;

#[derive(Clone)]
pub(super) struct AgentParts {
    host_overrides: Option<Arc<crate::agent::host_overrides::HostOverrides>>,
    /// `None` for booted contexts, which read the process live policy.
    policy: Option<Arc<crate::security::SecurityPolicy>>,
    approvals_disabled: bool,
    /// `None` resolves through the process registry.
    definitions: Option<Arc<crate::agent::harness::host_definition::AgentDefinitionRegistry>>,
    /// Shared by every turn context derived from the same agent context.
    state: Arc<super::super::agent_scope::AgentScopedState>,
}

impl Default for AgentParts {
    fn default() -> Self {
        Self {
            host_overrides: Some(Arc::new(
                crate::agent::host_overrides::HostOverrides::default(),
            )),
            policy: None,
            approvals_disabled: false,
            definitions: None,
            state: Default::default(),
        }
    }
}

impl AgentParts {
    /// The parts of a context derived from one owning `self` with `overlay`.
    /// An overlay that names an agent gets state slots of its own.
    pub(super) fn derive(&self, overlay: &mut ContextOverlay) -> Self {
        Self {
            host_overrides: overlay
                .host_overrides
                .take()
                .or_else(|| self.host_overrides.clone()),
            policy: overlay.agent_policy.take().or_else(|| self.policy.clone()),
            approvals_disabled: overlay.approvals_disabled || self.approvals_disabled,
            definitions: overlay
                .definitions
                .take()
                .or_else(|| self.definitions.clone()),
            state: if overlay.session_agent.is_some() || overlay.profile.is_some() {
                Default::default()
            } else {
                Arc::clone(&self.state)
            },
        }
    }
}

impl CoreContext {
    /// Agent-local host adapters inherited by this derived context.
    pub fn host_overrides(&self) -> Option<Arc<crate::agent::host_overrides::HostOverrides>> {
        self.agent.host_overrides.clone()
    }

    /// The security policy of the agent this context was derived for.
    pub fn agent_policy(&self) -> Option<Arc<crate::security::SecurityPolicy>> {
        self.agent.policy.clone()
    }

    /// Whether the interactive approval gate is off for this context's agent.
    pub fn approvals_disabled(&self) -> bool {
        self.agent.approvals_disabled
    }

    /// The sub-agent catalogue of the agent this context was derived for.
    pub fn definitions(
        &self,
    ) -> Option<Arc<crate::agent::harness::host_definition::AgentDefinitionRegistry>> {
        self.agent.definitions.clone()
    }

    /// The tenant (SaaS profile) this context serves, if any.
    pub fn profile(&self) -> Option<&str> {
        self.profile.as_deref()
    }

    /// Whether work beyond the caller's own handle still runs on this
    /// context's tenant: another `Arc` of this context (a request scope, a
    /// spawned task that captured it), or a turn context derived from it that
    /// shares its state slots ([`scope_with_turn_origin`](Self::scope_with_turn_origin),
    /// a `derive_with` naming neither agent nor profile). The caller is
    /// assumed to hold exactly one `Arc` of `self`.
    ///
    /// A host must not close or evict a tenant while this is `true`: its
    /// in-flight tables (turns, queues, caches) live in those slots.
    pub fn tenant_in_use(self: &Arc<Self>) -> bool {
        Arc::strong_count(self) > 1 || Arc::strong_count(&self.agent.state) > 1
    }

    /// The state slots this context owns.
    pub fn agent_state(&self) -> &super::super::agent_scope::AgentScopedState {
        &self.agent.state
    }
}
