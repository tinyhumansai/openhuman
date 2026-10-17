//! Per-thread inference-budget-exhaustion correlator (#3386).
//!
//! When a turn terminates with a budget-exhausted error we record a signal;
//! a *later* empty-response turn on the same thread and provider binding
//! consults it to upgrade the generic "empty response" copy into the
//! actionable out-of-credits copy.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use crate::core::runtime::DispatchCtx;

use super::state::key_for;

/// A recorded budget-exhausted signal: when it happened, and which provider
/// binding it happened on. The binding scopes the signal so a managed
/// out-of-credits error never mislabels a later empty turn the user has
/// re-routed to a different provider (local / BYO), whose balance is unrelated.
#[derive(Debug, Clone)]
struct BudgetSignal {
    provider_binding: String,
    at: Instant,
}

/// Per-thread "recent budget-exhausted" signal (issue #3386).
///
/// Set when a turn terminates with an inference budget-exhausted error; read by
/// a *later* turn on the same thread whose provider returned an empty 200. The
/// managed route closes the SSE cleanly under credit exhaustion (the response
/// already flushed HTTP 200, so there is no error frame and no inline budget
/// marker — `OpenHumanBilling` carries only `charged_amount_usd`). Without this
/// correlator such a budget-caused empty turn surfaces as the generic "empty
/// response" copy instead of the actionable out-of-credits copy.
///
/// The signal is scoped to the provider binding it was recorded on: budget is a
/// per-provider fact, so a managed-route exhaustion must not reclassify an empty
/// turn the thread has since re-routed to a local / BYO provider.
///
/// Kept in a sibling map rather than on `SessionEntry` so the signal survives
/// the de-poison session drop (an empty turn is not poisoned, but cold-boot
/// reseeds would otherwise be the wrong lifetime to hang this on).
fn thread_budget_signals(ctx: &DispatchCtx) -> Arc<ThreadBudgetSignals> {
    crate::core::runtime::slot_for::<ThreadBudgetSignals>(ctx)
}

type ThreadBudgetSignals = Mutex<HashMap<String, BudgetSignal>>;

/// How long a recorded budget-exhausted signal stays eligible to reclassify a
/// later empty turn on the same thread. Five minutes: long enough to bridge a
/// user retry after the first out-of-credits turn, short enough that a genuine
/// empty response well after the fact isn't mislabeled. A successful turn clears
/// the signal regardless (the balance is evidently usable again). See #3386.
const BUDGET_SIGNAL_TTL: Duration = Duration::from_secs(5 * 60);

/// Pure freshness predicate (age vs TTL), split out for clock-free testing.
fn budget_signal_is_fresh(age: Duration, ttl: Duration) -> bool {
    age <= ttl
}

/// Drop every expired entry from the map, not just the one being queried.
/// Without this, a thread that hits budget exhaustion and then never retries or
/// succeeds would leak its entry for the process lifetime. Called on the write
/// path so each new budget event sweeps the map.
fn prune_stale_budget_signals(signals: &mut HashMap<String, BudgetSignal>) {
    signals.retain(|_, sig| budget_signal_is_fresh(sig.at.elapsed(), BUDGET_SIGNAL_TTL));
}

/// Record that this thread just hit an inference budget-exhausted error on the
/// given provider binding.
pub(crate) async fn record_budget_signal(
    ctx: &DispatchCtx,
    thread_id: &str,
    provider_binding: &str,
) {
    let mut signals = thread_budget_signals(ctx).lock_owned().await;
    prune_stale_budget_signals(&mut signals);
    signals.insert(
        key_for(ctx, thread_id),
        BudgetSignal {
            provider_binding: provider_binding.to_string(),
            at: Instant::now(),
        },
    );
}

/// Clear any recorded budget signal for this thread — called on a successful
/// turn, where the balance is evidently usable again.
pub(crate) async fn clear_budget_signal(ctx: &DispatchCtx, thread_id: &str) {
    let mut signals = thread_budget_signals(ctx).lock_owned().await;
    signals.remove(&key_for(ctx, thread_id));
}

/// Whether this thread has a budget signal recorded within `BUDGET_SIGNAL_TTL`
/// **on the same provider binding** as the current turn. A binding mismatch or
/// an expired entry evicts it and reads as not-fresh, so a re-routed turn never
/// inherits the prior provider's exhaustion.
pub(crate) async fn has_fresh_budget_signal(
    ctx: &DispatchCtx,
    thread_id: &str,
    provider_binding: &str,
) -> bool {
    let mut signals = thread_budget_signals(ctx).lock_owned().await;
    let key = key_for(ctx, thread_id);
    match signals.get(&key) {
        Some(sig)
            if sig.provider_binding == provider_binding
                && budget_signal_is_fresh(sig.at.elapsed(), BUDGET_SIGNAL_TTL) =>
        {
            true
        }
        Some(_) => {
            signals.remove(&key);
            false
        }
        None => false,
    }
}

/// Test-only seeder: record a budget signal on `provider_binding` aged `age`
/// into the past so expiry can be exercised without sleeping.
#[cfg(test)]
async fn record_budget_signal_aged(
    ctx: &DispatchCtx,
    thread_id: &str,
    provider_binding: &str,
    age: Duration,
) {
    let mut signals = thread_budget_signals(ctx).lock_owned().await;
    let when = Instant::now().checked_sub(age).unwrap_or_else(Instant::now);
    signals.insert(
        key_for(ctx, thread_id),
        BudgetSignal {
            provider_binding: provider_binding.to_string(),
            at: when,
        },
    );
}

/// What the budget-correlator should do with a terminated turn (#3386).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BudgetCorrelation {
    /// The terminal error is itself an inference budget-exhausted error:
    /// record the signal and surface the budget copy.
    BudgetExhausted,
    /// An empty provider response coincided with a fresh same-thread budget
    /// signal: surface the budget copy in place of the "empty response" copy.
    UpgradeEmptyToBudget,
    /// No budget correlation — pass the error through unchanged.
    PassThrough,
}

/// Pure decision for the budget-correlator, split out so the branch matrix is
/// unit-testable without a clock or the full `run_chat_task` frame. The async
/// helpers above supply `has_fresh_signal`.
pub(crate) fn classify_budget_correlation(
    is_budget_error: bool,
    is_empty_response: bool,
    has_fresh_signal: bool,
) -> BudgetCorrelation {
    if is_budget_error {
        BudgetCorrelation::BudgetExhausted
    } else if is_empty_response && has_fresh_signal {
        BudgetCorrelation::UpgradeEmptyToBudget
    } else {
        BudgetCorrelation::PassThrough
    }
}

#[cfg(test)]
#[path = "../ops_budget_correlation_tests_tests.rs"]
mod budget_correlation_tests;
