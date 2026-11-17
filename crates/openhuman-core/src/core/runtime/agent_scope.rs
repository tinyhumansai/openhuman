//! Per-agent state owned by a [`CoreContext`].
//!
//! An embedded agent runs every turn under its own derived context
//! ([`CoreContext::derive_with`]). Core state that must not be shared between
//! agents hangs off that context instead of a process static:
//!
//! - [`AgentScopedState`]: a lazily populated, type-keyed slot map. Each
//!   derived agent context owns one; the booted default context owns its own,
//!   so the app and CLI keep a single instance as before.
//! - [`AgentContextRegistry`]: the live agent contexts by agent id, so work
//!   that starts outside a turn (cron) can run under the agent that owns it.
//! - [`agent_scope_dir`]: where an agent's workspace-scoped stores live.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, PoisonError, RwLock, Weak};

use super::CoreContext;

/// Type-keyed state slots owned by one context.
///
/// A slot is created on first use with `T::default()` and lives as long as
/// the context that owns it. Two contexts never share a slot.
#[derive(Default)]
pub struct AgentScopedState {
    slots: Mutex<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

impl AgentScopedState {
    /// The slot of type `T`, created on first use.
    pub fn slot<T: Default + Send + Sync + 'static>(&self) -> Arc<T> {
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = slots
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Arc::new(T::default()) as Arc<dyn Any + Send + Sync>);
        Arc::clone(entry)
            .downcast::<T>()
            .unwrap_or_else(|_| unreachable!("a slot is keyed by its own TypeId"))
    }

    /// Number of slots created so far.
    pub fn len(&self) -> usize {
        self.slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Whether no slot has been created yet.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Drops every slot.
    pub fn clear(&self) {
        self.slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }
}

impl std::fmt::Debug for AgentScopedState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentScopedState")
            .field("slots", &self.len())
            .finish()
    }
}

/// Slots used when no context exists at all.
static UNSCOPED: LazyLock<AgentScopedState> = LazyLock::new(AgentScopedState::default);

/// The slot of type `T` in the context `ctx` resolves to, or the unscoped
/// one when it resolves to none.
///
/// In SaaS a call with no bound context gets a throwaway slot: never the
/// shared unscoped one (which every user's lost work would meet in) and never
/// the operator's.
pub fn slot_for<T: Default + Send + Sync + 'static>(ctx: &super::DispatchCtx) -> Arc<T> {
    slot_in::<T>(super::is_saas(), ctx.resolved().as_deref())
}

/// [`slot_for`] as a function of the mode and the resolved context.
pub fn slot_in<T: Default + Send + Sync + 'static>(
    saas: bool,
    ctx: Option<&CoreContext>,
) -> Arc<T> {
    match ctx {
        Some(ctx) => ctx.agent_state().slot::<T>(),
        None if saas => {
            log::warn!(
                "[core-context] unscoped state slot {} in SaaS; handing out a throwaway one",
                std::any::type_name::<T>()
            );
            Arc::new(T::default())
        }
        None => UNSCOPED.slot::<T>(),
    }
}

static AGENT_CONTEXTS: LazyLock<RwLock<HashMap<String, Weak<CoreContext>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// The process's live agent contexts, by agent id.
///
/// Holds weak references: registering a context does not keep it alive, and a
/// dropped agent's entry resolves to `None` even before it is deregistered.
pub struct AgentContextRegistry;

impl AgentContextRegistry {
    /// Records `ctx` as the context of `agent_id`, replacing any dead entry.
    pub fn register(agent_id: &str, ctx: &Arc<CoreContext>) {
        {
            let mut map = AGENT_CONTEXTS
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            map.retain(|_, weak| weak.strong_count() > 0);
            map.insert(agent_id.to_string(), Arc::downgrade(ctx));
            log::debug!(
                "[core-context] agent context registered agent={agent_id} live={}",
                map.len()
            );
        }
        // With a storage backend, remember the agent there so a restarted
        // process still visits its records (`crate::storage::agents`). After
        // the lock is released: recording waits on the storage bridge.
        crate::storage::agents::record(agent_id);
    }

    /// Removes `agent_id`'s entry when it still points at `ctx`.
    pub fn deregister(agent_id: &str, ctx: &Arc<CoreContext>) -> bool {
        let mut map = AGENT_CONTEXTS
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        let owned = map
            .get(agent_id)
            .is_some_and(|weak| std::ptr::eq(weak.as_ptr(), Arc::as_ptr(ctx)));
        if owned {
            map.remove(agent_id);
            log::debug!("[core-context] agent context deregistered agent={agent_id}");
        }
        owned
    }

    /// The live context of `agent_id`, if that agent is still alive.
    pub fn get(agent_id: &str) -> Option<Arc<CoreContext>> {
        AGENT_CONTEXTS
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(agent_id)
            .and_then(Weak::upgrade)
    }

    /// Every live agent context, sorted by agent id.
    pub fn live() -> Vec<(String, Arc<CoreContext>)> {
        let mut live: Vec<(String, Arc<CoreContext>)> = AGENT_CONTEXTS
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter_map(|(id, weak)| weak.upgrade().map(|ctx| (id.clone(), ctx)))
            .collect();
        live.sort_by(|a, b| a.0.cmp(&b.0));
        live
    }
}

/// The directory an agent's workspace-scoped stores live under:
/// `<workspace>/agents/<id>` under a context derived for an agent, else the
/// workspace itself.
pub fn agent_scope_dir(config: &crate::config::Config) -> PathBuf {
    match CoreContext::current().and_then(|ctx| ctx.session_agent().map(str::to_owned)) {
        Some(agent_id) => config.workspace_dir.join("agents").join(agent_id),
        None => config.workspace_dir.clone(),
    }
}

/// The agent id of the ambient context, when it was derived for one.
///
/// Genuinely per-agent only: a SaaS profile's default agent has none, so this
/// is never a tenant key. Tables and stores that must keep tenants apart key
/// on [`current_tenant`](super::current_tenant) instead; the remaining callers
/// only log it.
pub fn current_agent_id() -> Option<String> {
    CoreContext::current().and_then(|ctx| ctx.session_agent().map(str::to_owned))
}

/// A context derived from `parent` for `agent_id`, for tests that need two
/// agents side by side.
#[cfg(test)]
pub(crate) fn test_agent_context(parent: &Arc<CoreContext>, agent_id: &str) -> Arc<CoreContext> {
    parent.derive_with(
        super::ContextOverlay::new(
            crate::config::Config::default(),
            super::DomainSet::kernel(),
            crate::tools::toolpacks::ToolGroups::none(),
        )
        .session_agent(agent_id),
    )
}

#[cfg(test)]
#[path = "agent_scope_tests.rs"]
mod tests;
