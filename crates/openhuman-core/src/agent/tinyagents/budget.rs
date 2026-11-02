//! Host budget policy, handed to the turn once at admission and carried
//! explicitly to children (`HostTurnEnv::model_budget` -> `OpenHumanRunContext`).

use std::sync::Arc;

/// Product recursion ceiling shared by native and MCP delegation.
pub(crate) const MAX_SPAWN_DEPTH: usize = 3;
pub use tinyagents_harness::cost::{
    ControlledDispatchError, DispatchModelOptions, DispatchRefusal, DispatchRefusalReason,
    PhysicalDispatchAdmission, PhysicalDispatchPermit,
};
pub use tinyinference_llm::model::budget::{
    Budget, BudgetExceeded, BudgetSnapshot, CallBudget, Spend, SpendLimits,
};

/// Effective host depth ceiling, never wider than product policy.
pub(crate) fn depth(context: &super::host::OpenHumanRunContext) -> usize {
    context
        .max_spawn_depth
        .unwrap_or(crate::agent::tinyagents::budget::MAX_SPAWN_DEPTH)
        .min(crate::agent::tinyagents::budget::MAX_SPAWN_DEPTH)
}
pub(super) fn install_depth(
    harness: &mut tinyagents_harness::runtime::AgentHarness<(), super::host::OpenHumanRunContext>,
    context: &super::host::OpenHumanRunContext,
) {
    if let Some(max_depth) = context.max_spawn_depth {
        let mut policy = harness.policy().clone();
        policy.limits.max_depth = policy.limits.max_depth.min(max_depth);
        harness.with_policy(policy);
    }
}

impl crate::inference::turn_models::TurnModels {
    /// Admit and charge every concrete route and summarizer below harness
    /// retries/fallback, and below attachment rendering where it is present.
    pub(crate) fn with_host_controls(
        mut self,
        budget: Option<&tinyagents_harness::cost::ModelBudget>,
        admission: Option<Arc<dyn PhysicalDispatchAdmission>>,
    ) -> Self {
        let Some(controls) = crate::inference::host_controls::HostControls::new(budget, admission)
        else {
            return self;
        };
        if let Some(slot) = &self.host_controls {
            tracing::debug!(
                budgeted = budget.is_some(),
                "[budget] installing turn controls beneath attachment rendering"
            );
            slot.install(controls);
            return self;
        }
        self.primary = controls.wrap(self.primary);
        self.summarizer = controls.wrap(self.summarizer);
        for (_, route) in &mut self.routes {
            *route = controls.wrap(route.clone());
        }
        self
    }
}
