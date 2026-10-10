//! Live, agent-owned host adapters inherited by derived turn contexts.
//!
//! These are OpenHuman's injection points into its session builder. The model
//! contract remains TinyInference's, and the session-store contract remains
//! TinyAgents'. Credentials and the event bus are deliberately not overridden.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::agent::hooks::{PostTurnHook, ToolHook};
use tinyinference_llm::model::ChatModel;

pub use tinyagents_harness::cancel::CancellationToken;

/// Runtime or agent host adapters, shared by every context derived from it.
#[derive(Default)]
pub struct HostOverrides {
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
    pub session_store: Option<Arc<dyn crate::agent::session_store::SessionStoreProvider>>,
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

tokio::task_local! { static TURN_CANCELLATION: CancellationToken; }

/// Run a turn with a caller-owned cancellation token, shared with child runs.
pub async fn with_cancellation<F: std::future::Future>(
    token: CancellationToken,
    future: F,
) -> F::Output {
    TURN_CANCELLATION.scope(token, future).await
}

/// The caller's token, or a fresh token for entrypoints without one.
pub fn current_cancellation() -> CancellationToken {
    TURN_CANCELLATION.try_with(Clone::clone).unwrap_or_default()
}

#[cfg(test)]
#[path = "host_overrides_tests.rs"]
mod tests;
