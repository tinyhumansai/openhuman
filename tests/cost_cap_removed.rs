//! The spend cap is gone: a spend state that used to refuse must now grant.
//!
//! This is a reproduction of the reported symptom, not a test of the diff. The
//! user's installed app emitted, on every attempt:
//!
//! ```text
//! [tinyagents::mw] cost budget exceeded — failing before model call
//!     current_usd=10.0341857 limit_usd=10 period=Day
//! ```
//!
//! and surfaced it as "Something went wrong. Please try again." — advice that
//! could not work, because the refusal happened *before* the model call, so
//! every retry hit the same wall.
//!
//! It lives in its own integration binary rather than beside the module for one
//! reason: `GLOBAL_TRACKER` is a process-wide `OnceCell`
//! (`platform/cost/global.rs`). A unit test that installs a global tracker wins
//! or loses depending on which test in the shared `--lib` process got there
//! first, so the reproduction would be order-dependent — and an order-dependent
//! reproduction that happens to find `None` proves nothing, because the gate
//! treats "no tracker" as "no budget opinion" and grants. One test per process
//! is what makes the spend state below actually reach the code under test.

use std::sync::Arc;

use openhuman_core::agent::tinyagents::host::OpenHumanBudgetGate;
use openhuman_core::config::Config;
use openhuman_core::platform::cost::route::CostRoute;
use openhuman_core::platform::cost::{self, TokenUsage};
use tinyagents_harness::host::budget_gate::{BudgetGate, CallEstimate};

/// Spend far past every limit the cap ever enforced: the retired daily default
/// was $10 and the monthly default is $100.
const RUNAWAY_SPEND_USD: f64 = 500.0;

#[tokio::test]
async fn a_spend_state_that_used_to_refuse_now_grants_and_is_still_recorded() {
    let tmp = tempfile::TempDir::new().expect("tempdir");

    // Enforcement-era config: `enabled = true` is what used to arm the cap.
    let mut config = Config::default();
    config.cost.enabled = true;
    cost::init_global(config.cost.clone(), tmp.path());
    let tracker = cost::try_global().expect(
        "this test owns its process, so nothing else can have claimed the \
         global tracker first; without it the gate has no budget opinion and \
         the reproduction below would be vacuous",
    );

    // Managed route — only managed spend was ever counted against the cap
    // (#5016), so a BYOK model here would have been ignored even before the
    // removal and would prove nothing.
    let managed_test_model = "openrouter/test/managed-model";
    assert_eq!(
        cost::route::route_for_model(managed_test_model),
        CostRoute::Managed,
        "the fixture must exercise managed spend"
    );
    let mut usage = TokenUsage::new(managed_test_model, 10_000, 5_000, 1.0, 2.0);
    usage.cost_usd = RUNAWAY_SPEND_USD;
    tracker.record_usage(usage).expect("recording the spend");

    // 1. THE SYMPTOM. `acquire()` is the surviving enforcement point
    //    (`host/budget_gate.rs`), the one that produces the near-identical
    //    "[tinyagents][budget] refusing model call" message. Before the
    //    removal this returned `Err(LimitExceeded)` for exactly this state.
    let gate = OpenHumanBudgetGate::new(Arc::new(config));
    let estimate = CallEstimate::new(managed_test_model, 1_000, 1_000).with_agent("lead");
    match gate.acquire(&estimate).await {
        Ok(permit) => drop(permit),
        Err(err) => panic!(
            "a runaway ${RUNAWAY_SPEND_USD} of managed spend must no longer refuse \
             a model call: the budget cap was removed, and this is the \
             enforcement point that used to refuse before the call was ever \
             made. Got: {err:?}"
        ),
    }

    // 2. WHAT MUST SURVIVE. Removing the cap must not blind the spend view.
    //    The dashboard reads the same records the cap used to gate on, so
    //    assert the spend is still there rather than assuming it, since
    //    "I didn't touch it" is not evidence.
    let summary = tracker.get_summary().expect("summary");
    assert!(
        (summary.session_cost_usd - RUNAWAY_SPEND_USD).abs() < 0.0001,
        "the spend must still be recorded for the dashboard after the cap is \
         gone; got {} USD",
        summary.session_cost_usd
    );
    assert_eq!(
        summary.request_count, 1,
        "the recorded request must still be counted for the dashboard"
    );
}
