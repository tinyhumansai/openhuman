//! The dispatch context of a bus event.
//!
//! A subscriber runs on the broker's task, not on the publisher's, so it
//! inherits no task scope. An event that concerns one tenant therefore carries
//! that tenant ([`Tenant`]), and the subscriber rebuilds its [`DispatchCtx`]
//! from it with [`DispatchCtx::for_event`]:
//!
//! - the tenant's live context when it has one (a SaaS profile's, an embedded
//!   agent's), whatever other context happens to be active;
//! - the process default for an untagged event on a single-user host;
//! - nothing (the subscriber logs and skips) in SaaS for an untagged event, and
//!   for a tagged one whose tenant is gone: work that lost its tenant never
//!   acts as the operator.

use std::sync::Arc;

use super::{CoreContext, DispatchCtx, Tenant};

/// The live context `tenant` acts under, if it has one.
///
/// A SaaS profile resolves through the process's profile host; an embedded
/// agent through the agent context registry (outside SaaS it may also fall
/// back to the default context acting for it). A tenant with neither profile
/// nor agent is the host's own, which has no context of its own to find.
pub fn context_for_tenant(tenant: &Tenant) -> Option<Arc<CoreContext>> {
    if let Some(profile) = tenant.profile.as_deref() {
        let id = crate::profiles::ProfileId::parse(profile).ok()?;
        return crate::profiles::host::host()?
            .get(&id)
            .map(|state| Arc::clone(state.context()));
    }
    let agent = tenant.agent.as_deref()?;
    crate::storage::agents::context_for(agent)
}

impl DispatchCtx {
    /// The tenant this call acts for, to tag an event with. `None` when it has
    /// none (SaaS work that lost its scope): the event then carries no tenant
    /// and its subscribers fail closed.
    pub fn event_tenant(&self) -> Option<Tenant> {
        self.tenant().ok()
    }

    /// The context a subscriber of an event tagged with `tenant` runs under.
    ///
    /// `None` means the event must not be handled: SaaS with no tenant, or a
    /// tenant (profile or agent) with no live context left. See the module
    /// docs.
    pub fn for_event(tenant: Option<&Tenant>) -> Option<Self> {
        let Some(tenant) = tenant.filter(|tenant| tenant.is_scoped()) else {
            // Untagged, or tagged with the host's own default orchestrator.
            return if super::is_saas() {
                None
            } else {
                Some(Self::process_default())
            };
        };
        match context_for_tenant(tenant) {
            Some(core) => Some(Self::new(core)),
            None => {
                tracing::warn!(
                    profile = ?tenant.profile,
                    agent = ?tenant.agent,
                    "[event-ctx] no live context for the event's tenant; event skipped"
                );
                None
            }
        }
    }
}

#[cfg(test)]
#[path = "event_ctx_tests.rs"]
mod tests;
