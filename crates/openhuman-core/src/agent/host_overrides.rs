//! Live, agent-owned host adapters inherited by derived turn contexts.
//!
//! These are OpenHuman's injection points into its session builder. The model
//! contract remains TinyInference's, and the session-store contract remains
//! TinyAgents'. Credentials and the event bus are deliberately not overridden.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::agent::hooks::{PostTurnHook, ToolHook};
use tinyinference_llm::model::ChatModel;

/// Runtime or agent host adapters, shared by every context derived from it.
#[derive(Default)]
pub struct HostOverrides {
    /// Immutable operator capabilities for an authenticated hosted profile.
    pub profile_capabilities: Option<Arc<crate::profiles::capabilities::ProfileCapabilities>>,
    /// Parent runtime adapters; local hook registration never edits this parent.
    pub parent: Option<Arc<HostOverrides>>,
    /// Approval registration barrier owned by this agent instance.
    pub approval_scope: Option<Arc<crate::security::approval::ApprovalScope>>,
    post_turn_hooks: RwLock<BTreeMap<String, Arc<dyn PostTurnHook>>>,
    tool_hooks: RwLock<BTreeMap<String, Arc<dyn ToolHook>>>,
    /// Native inference used by this agent instead of config-routed inference.
    pub model: Option<Arc<dyn ChatModel<()>>>,
    /// Role-specific native models; absent roles use `model` or config routing.
    pub role_models: BTreeMap<String, Arc<dyn ChatModel<()>>>,
    /// The agent's durable session store, ahead of the runtime's store.
    pub session_store: Option<Arc<dyn tinyagents_session::port::SessionStoreProvider>>,
    /// The tools the agent's definition names, when its scope is a named
    /// list: what lets an embedded agent have one of the
    /// [`OPERATOR_ADMITTABLE`](crate::profiles::tools::OPERATOR_ADMITTABLE)
    /// tools although its domain family is off.
    pub named_tools: Option<Arc<std::collections::HashSet<String>>>,
}

impl std::fmt::Debug for HostOverrides {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostOverrides")
            .field("has_model", &self.model.is_some())
            .field("model_roles", &self.role_models.keys().collect::<Vec<_>>())
            .field("has_session_store", &self.session_store.is_some())
            .finish_non_exhaustive()
    }
}

impl HostOverrides {
    /// Resolve inherited profile adapters without installing process globals.
    pub fn profile_capabilities(
        &self,
    ) -> Option<Arc<crate::profiles::capabilities::ProfileCapabilities>> {
        self.profile_capabilities.clone().or_else(|| {
            self.parent
                .as_ref()
                .and_then(|parent| parent.profile_capabilities())
        })
    }

    /// The named tool scope of this agent, or of the nearest parent that has
    /// one.
    pub fn named_tools(&self) -> Option<Arc<std::collections::HashSet<String>>> {
        self.named_tools
            .clone()
            .or_else(|| self.parent.as_ref().and_then(|parent| parent.named_tools()))
    }

    /// Resolve the instance barrier inherited by derived turn contexts.
    pub fn approval_scope(&self) -> Option<Arc<crate::security::approval::ApprovalScope>> {
        self.approval_scope
            .clone()
            .or_else(|| self.parent.as_ref().and_then(|p| p.approval_scope()))
    }

    /// Add or replace an agent-local post-turn hook by name; `None` removes it.
    pub fn post_turn_hook(&self, name: &str, hook: Option<Arc<dyn PostTurnHook>>) {
        let mut hooks = self
            .post_turn_hooks
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        hooks.remove(name);
        if let Some(hook) = hook {
            hooks.insert(name.to_owned(), hook);
        }
    }

    /// Add or replace an agent-local tool hook by name; `None` removes it.
    pub fn tool_hook(&self, name: &str, hook: Option<Arc<dyn ToolHook>>) {
        let mut hooks = self
            .tool_hooks
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        hooks.remove(name);
        if let Some(hook) = hook {
            hooks.insert(name.to_owned(), hook);
        }
    }

    /// Snapshot the agent-local post-turn hooks for a new session.
    pub fn post_turn_hooks(&self) -> Vec<Arc<dyn PostTurnHook>> {
        let mut hooks = self
            .parent
            .as_ref()
            .map(|parent| parent.post_turn_hooks())
            .unwrap_or_default();
        hooks.extend(
            self.post_turn_hooks
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .values()
                .cloned(),
        );
        hooks
    }

    /// Snapshot the agent-local tool hooks for a new turn.
    pub fn tool_hooks(&self) -> Vec<Arc<dyn ToolHook>> {
        let mut hooks = self
            .parent
            .as_ref()
            .map(|parent| parent.tool_hooks())
            .unwrap_or_default();
        hooks.extend(
            self.tool_hooks
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .values()
                .cloned(),
        );
        hooks
    }

    /// Resolve a native model for a workload role without touching credentials.
    pub fn model_for(&self, role: &str) -> Option<Arc<dyn ChatModel<()>>> {
        self.role_models
            .get(role)
            .cloned()
            .or_else(|| self.model.clone())
    }
}

#[cfg(test)]
#[path = "host_overrides_tests.rs"]
mod tests;
