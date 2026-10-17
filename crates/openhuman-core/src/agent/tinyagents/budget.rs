//! Host budget policy, handed to the turn once at admission and carried
//! explicitly to children (`HostTurnEnv::model_budget` -> `OpenHumanRunContext`).
use std::sync::Arc;

/// Product recursion ceiling shared by native and MCP delegation.
pub(crate) const MAX_SPAWN_DEPTH: usize = 3;
pub use tinyinference_llm::model::budget::{
    Budget, BudgetExceeded, BudgetSnapshot, CallBudget, Spend, SpendLimits,
};
use tinyinference_llm::model::ChatModel;

/// A shared ledger and conservative reservation bounds for every physical call.
#[derive(Debug, Clone)]
pub struct ModelBudget {
    /// Run/turn ledger. Children of a turn share it.
    pub ledger: Budget,
    /// Per-call input, output and charge upper bounds for all allowed routes.
    pub call: CallBudget,
}
impl ModelBudget {
    /// Wait for live reservations when settled spend still affords admission.
    /// Clones and child turns inherit this opt-in; internal retries stay fail-fast.
    pub fn wait_for_capacity(mut self) -> Self {
        self.ledger = self.ledger.wait_for_capacity();
        self
    }

    /// Apply the policy below retries and fallback selection.
    pub(crate) fn wrap(&self, model: Arc<dyn ChatModel<()>>) -> Arc<dyn ChatModel<()>> {
        Arc::new(tinyinference_llm::model::budget::BudgetedModel::new(
            Arc::new(tinyagents_harness::cost::GatewayChargeModel::new(model)),
            self.ledger.clone(),
            self.call,
        ))
    }
}
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

impl super::turn_models::TurnModels {
    /// Wrap every concrete route and summarizer below harness retries/fallback.
    pub(crate) fn with_budget(mut self, budget: Option<&ModelBudget>) -> Self {
        let Some(budget) = budget else {
            return self;
        };
        self.primary = budget.wrap(self.primary);
        self.summarizer = budget.wrap(self.summarizer);
        for (_, route) in &mut self.routes {
            *route = budget.wrap(route.clone());
        }
        self
    }
}
