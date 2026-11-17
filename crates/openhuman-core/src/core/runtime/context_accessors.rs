use super::CoreContext;

impl CoreContext {
    /// Source binding for autonomy-tier decisions made by long-lived adapters.
    pub fn autonomy_tier_source(&self) -> super::AutonomyTierSource {
        self.autonomy_tier_source
    }

    /// The fixed autonomy ceiling inherited by this context, if any.
    pub fn autonomy_tier_ceiling(&self) -> Option<crate::security::AutonomyLevel> {
        self.autonomy_tier_ceiling
    }

    /// The hook registrations bound to this context. Shared (not copied) by
    /// derived contexts.
    pub fn extensions(&self) -> &crate::core::slots::Extensions {
        &self.extensions
    }

    /// The hook registrations of the context this call runs under, for
    /// [`HookSlot`](crate::core::slots::HookSlot) lookups.
    ///
    /// Outside SaaS that is the ambient context (the process default when the
    /// task has no scope). In SaaS only the task's own scope counts: a task that
    /// lost its scope must not read the operator's registrations.
    pub fn current_extensions() -> Option<crate::core::slots::Extensions> {
        let ctx = if crate::core::runtime::is_saas() {
            Self::scoped()
        } else {
            Self::current()
        };
        ctx.map(|ctx| ctx.extensions.clone())
    }
}
