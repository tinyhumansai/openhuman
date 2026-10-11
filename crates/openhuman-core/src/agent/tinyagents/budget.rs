//! Host budget policy, scoped once at admission and carried explicitly to children.
use std::sync::Arc;
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
            Arc::new(super::budget_charge::GatewayChargeModel::new(model)),
            self.ledger.clone(),
            self.call,
        ))
    }
}
tokio::task_local! { static MODEL_BUDGET: ModelBudget; }
/// Scope host policy around turn admission; the explicit run carrier copies it.
pub async fn with_budget<F: std::future::Future>(budget: ModelBudget, future: F) -> F::Output {
    MODEL_BUDGET.scope(budget, Box::pin(future)).await
}
pub(crate) fn current() -> Option<ModelBudget> {
    MODEL_BUDGET.try_with(Clone::clone).ok()
}

// Fanout leaves narrow the existing host depth policy at admission.
tokio::task_local! { static SPAWN_DEPTH_LIMIT: usize; }
/// Run a typed fanout call with a narrower model-directed child-depth ceiling.
pub async fn with_spawn_depth_limit<F: std::future::Future>(limit: usize, future: F) -> F::Output {
    SPAWN_DEPTH_LIMIT.scope(limit, Box::pin(future)).await
}
pub(crate) fn spawn_depth_limit() -> Option<usize> {
    SPAWN_DEPTH_LIMIT.try_with(|limit| *limit).ok()
}

/// Effective host depth ceiling, never wider than product policy.
pub(crate) fn depth(context: &super::host::OpenHumanRunContext) -> usize {
    context
        .max_spawn_depth
        .unwrap_or(crate::agent::harness::MAX_SPAWN_DEPTH)
        .min(crate::agent::harness::MAX_SPAWN_DEPTH)
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
