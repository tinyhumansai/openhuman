//! Enforced shared budgets for completion calls, tool loops and synchronous children.
//!
//! Attach [`ModelBudget`] to a turn or completer. Use [`Budget::child`] for a
//! per-turn ledger charged to a shared review/run ledger. Each concrete route
//! and summarizer reserves before calling its provider, including every retry.
//! Unknown usage, errors and cancellation consume the full reservation. Cost
//! bounds are host-supplied upper bounds for every allowed route, not billing
//! estimates; providers cannot be forced to honour a monetary cap locally.
//!
//! Admission is fail-fast by default. [`ModelBudget::wait_for_capacity`] opts
//! a policy into waiting only when live reservations prevent an otherwise
//! affordable call; clones and child turns inherit it. Calls that fit still
//! run concurrently, while settled spend that cannot fit refuses immediately.
//! Canceling before admission reserves nothing. Internal transport retries
//! remain fail-fast, and unknown dispatched charges are never refunded.
//! Hosts retain their existing logical deadlines and cancellation controls.
//!
//! Budgeted calls currently accept text only. Input bounds use serialized-byte
//! counts conservatively; multimodal requests fail before dispatch. Choose
//! bounds that include model framing and the maximum price on allowed routes.
pub use openhuman_core::agent::tinyagents::budget::{
    Budget, BudgetExceeded, BudgetSnapshot, CallBudget, ModelBudget, Spend, SpendLimits,
};
