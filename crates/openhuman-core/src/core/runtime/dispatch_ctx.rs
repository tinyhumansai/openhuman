//! The explicit context a controller handler runs under.
//!
//! Every registered controller handler is `fn(DispatchCtx, Map) -> ControllerFuture`.
//! The transport that accepted the call (the JSON-RPC server, the CLI, the
//! device tunnel, [`CoreRuntime::invoke_in`](super::CoreRuntime::invoke_in))
//! builds the [`DispatchCtx`] and hands it to
//! [`invoke_method`](crate::core::invoke::invoke_method); the registry passes it
//! on to the handler. Nothing in between reads it from task state, so two
//! concurrent dispatches for different tenants cannot see each other's value.
//!
//! A handler hands the context to whatever it calls (`ctx.core()` for the
//! [`CoreContext`], or the helpers built on it) rather than reaching for an
//! ambient one.

use std::sync::Arc;

use super::CoreContext;

/// The per-call context a controller handler receives.
///
/// Cheap to clone (one `Arc`). It is `unbound` only when no [`CoreContext`]
/// exists at all: a unit test that never booted the core, or a call made
/// before boot. A booted host always dispatches with a bound context.
#[derive(Clone, Default)]
pub struct DispatchCtx {
    core: Option<Arc<CoreContext>>,
}

impl DispatchCtx {
    /// Dispatch under `core`: the tenant's context for a SaaS request, an
    /// agent's derived context for an embedded call, the host's own otherwise.
    pub fn new(core: Arc<CoreContext>) -> Self {
        Self { core: Some(core) }
    }

    /// Dispatch with no context. Only for tests and pre-boot callers; every
    /// reader of the context treats this as "nothing configured here".
    pub fn unbound() -> Self {
        Self { core: None }
    }

    /// Dispatch under the process default context (the first one built), or
    /// unbound before any exists. What the CLI and the desktop shell use: a
    /// single-user host has exactly one tenant.
    pub fn process_default() -> Self {
        Self {
            core: CoreContext::default_context(),
        }
    }

    /// The ambient context of the calling task, or unbound. In SaaS only the
    /// task's own scope counts.
    ///
    /// Transitional (#7801): for a caller that has no context of its own to
    /// hand over yet. Every use is a reader still to be moved onto an explicit
    /// carrier, and the function goes away with the ambient context.
    pub fn ambient() -> Self {
        // SaaS reads only the task's own scope, never the process default, so
        // work that lost its tenant fails closed instead of acting as the operator.
        let core = if super::is_saas() {
            CoreContext::scoped()
        } else {
            CoreContext::current()
        };
        Self { core }
    }

    /// The context for work that arrives with none of its own and no tenant on
    /// the wire: a socket-driven bus event, a module host callback, a shutdown
    /// hook. Outside SaaS that is the process default, the one context a
    /// single-user host has. In SaaS it is unbound, so work that lost its
    /// tenant fails closed instead of acting as the operator. It never reads
    /// the calling task's scope; an event that names its tenant uses
    /// [`for_event`](Self::for_event), a caller holding a context passes it.
    pub fn host_default() -> Self {
        Self::host_default_in(super::is_saas())
    }

    /// [`host_default`](Self::host_default) as a pure function of the mode, so
    /// the SaaS rule is testable without locking the test process to SaaS.
    pub(crate) fn host_default_in(saas: bool) -> Self {
        if saas {
            Self::unbound()
        } else {
            Self::process_default()
        }
    }

    /// The context readers act under: the bound one, else (outside SaaS) the
    /// process default, else none.
    ///
    /// An unbound context outside SaaS stands for the host's one context, as an
    /// unscoped task always did, so tests and pre-boot callers that never bound
    /// one read the default. In SaaS an unbound context stays `None`: work that
    /// lost its tenant must fail closed rather than act as the operator.
    pub fn resolved(&self) -> Option<Arc<CoreContext>> {
        match &self.core {
            Some(core) => Some(Arc::clone(core)),
            None if super::is_saas() => None,
            None => CoreContext::default_context(),
        }
    }

    /// The tenant this call acts for: the resolved context's profile and
    /// agent. Outside SaaS an unresolved context is the default tenant; in SaaS
    /// it is [`NoTenant`](super::NoTenant), so the caller fails closed.
    pub fn tenant(&self) -> Result<super::Tenant, super::NoTenant> {
        super::tenant::tenant_in(super::is_saas(), self.resolved().as_deref())
    }

    /// The configuration an embedder supplied for the context this call runs
    /// under, if any. See [`CoreContext::embedder_config`].
    pub fn embedder_config(&self) -> Option<crate::config::Config> {
        self.resolved()
            .and_then(|core| core.embedder_config().cloned())
    }

    /// Loads the config this call runs against: the bound context's embedder
    /// config when it has one, else the persisted config. Fails closed in
    /// SaaS when no context is bound. See
    /// [`load_config_in`](crate::config::ops::load_config_in).
    pub async fn load_config(&self) -> Result<crate::config::Config, String> {
        crate::config::ops::load_config_in(self).await
    }

    /// The calling task's scoped context including a bare one, which carries a
    /// turn origin and nothing else (a unit test or library caller that never
    /// booted a core).
    ///
    /// Transitional (#7801), like [`ambient`](Self::ambient): for a reader of
    /// the turn origin that has no carrier of its own yet. Unlike `ambient` it
    /// never falls back to the process default, which supplies no authority.
    pub fn ambient_turn() -> Self {
        Self {
            core: CoreContext::scoped_for_turn(),
        }
    }

    /// The `[components]` switches of the context this call runs under. No
    /// context at all (a bare library call, pre-boot) means nothing narrowed
    /// them, so every component is on.
    pub fn components(&self) -> super::ComponentSet {
        self.resolved()
            .map_or_else(super::ComponentSet::all, |core| core.components())
    }

    /// The domain families live for the context this call runs under, or
    /// `None` when there is no context (callers read that as "unfiltered").
    pub fn domains(&self) -> Option<super::DomainSet> {
        self.resolved().map(|core| core.domains())
    }

    /// Host adapters of the agent this call runs for.
    pub fn host_overrides(&self) -> Option<Arc<crate::agent::host_overrides::HostOverrides>> {
        self.resolved().and_then(|core| core.host_overrides())
    }

    /// The security policy of the agent this call runs for, if it carries one.
    pub fn agent_policy(&self) -> Option<Arc<crate::security::SecurityPolicy>> {
        self.resolved().and_then(|core| core.agent_policy())
    }

    /// Whether the interactive approval gate is off for the agent this call
    /// runs for.
    pub fn approvals_disabled(&self) -> bool {
        self.resolved()
            .is_some_and(|core| core.approvals_disabled())
    }

    /// The origin of the turn this call belongs to. Only a bound context
    /// supplies one: the process default never carries authority, so an
    /// unbound call answers `None` and the gate reads it as `Unknown`.
    pub fn turn_origin(&self) -> Option<crate::agent::turn_origin::AgentTurnOrigin> {
        self.core
            .as_ref()
            .and_then(|core| core.turn_origin().cloned())
    }

    /// [`turn_origin`](Self::turn_origin) of this call, else the origin the
    /// calling task's turn scope carries. For a handler whose context was
    /// built by a transport that does not carry the turn's origin, while a
    /// turn that called it into being still holds it (transitional, #7801).
    pub fn turn_origin_or_scoped(&self) -> Option<crate::agent::turn_origin::AgentTurnOrigin> {
        self.turn_origin()
            .or_else(|| Self::ambient_turn().turn_origin())
    }
    /// This context with `origin` bound as its turn origin, for a run that
    /// acts under a trusted entry point of its own (a saved flow). The base is
    /// this context, else the calling task's turn scope, the process default,
    /// or a bare one, the same resolution `agent::turn_origin::with_origin`
    /// scopes under. This context is not mutated.
    pub fn with_turn_origin(&self, origin: crate::agent::turn_origin::AgentTurnOrigin) -> Self {
        let base = self
            .core
            .clone()
            .or_else(CoreContext::scoped_for_turn)
            .or_else(CoreContext::default_context)
            .unwrap_or_else(CoreContext::bare);
        Self::new(base.derive_channel_turn(None, Some(origin)))
    }

    /// A non-owning handle on the bound context, for state that outlives the
    /// call (a session, a built turn). The tenant lease is the strong count of
    /// the context ([`CoreContext::tenant_in_use`]), so a long-lived holder
    /// must not keep it alive.
    pub fn downgrade(&self) -> Option<std::sync::Weak<CoreContext>> {
        self.core.as_ref().map(Arc::downgrade)
    }

    /// Dispatch under the context `weak` still points at, else the calling
    /// task's ambient one (a holder that was never bound, or whose tenant is
    /// gone).
    pub fn from_weak(weak: Option<&std::sync::Weak<CoreContext>>) -> Self {
        match weak.and_then(std::sync::Weak::upgrade) {
            Some(core) => Self::new(core),
            None => Self::ambient(),
        }
    }

    /// The context this call runs under, if any.
    pub fn core(&self) -> Option<&Arc<CoreContext>> {
        self.core.as_ref()
    }

    /// Takes the context out, for a caller that spawns a task and must hand
    /// it over explicitly.
    pub fn into_core(self) -> Option<Arc<CoreContext>> {
        self.core
    }
}

impl From<Arc<CoreContext>> for DispatchCtx {
    fn from(core: Arc<CoreContext>) -> Self {
        Self::new(core)
    }
}

impl std::fmt::Debug for DispatchCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DispatchCtx")
            .field("bound", &self.core.is_some())
            .finish()
    }
}

#[cfg(test)]
#[path = "dispatch_ctx_tests.rs"]
mod tests;
