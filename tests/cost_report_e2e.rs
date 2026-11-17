//! Cost reporting and budget refusal end to end: the `cost_report` and
//! `cost_cache_report` RPC methods over a seeded ledger, and a single-user
//! `[[cost.budgets]]` refuse policy turning a model call into `BUDGET_EXCEEDED`.
//!
//! Own integration binary, one test: the cost tracker is a process-wide global
//! and the config is found through `OPENHUMAN_WORKSPACE`, so the phases below
//! must share one process and run in order.

use openhuman_core::core::runtime::DispatchCtx;
use std::sync::Arc;

use chrono::{Datelike, Duration, Utc};
use openhuman_core::agent::tinyagents::host::OpenHumanBudgetGate;
use openhuman_core::config::Config;
use openhuman_core::core::invoke::{default_state, invoke_method};
use openhuman_core::platform::cost::{self, TokenUsage};
use serde_json::{json, Value};
use tinyagents_harness::host::budget_gate::{BudgetGate, CallEstimate};

/// One model call: `(model, thread, input tokens, cached input tokens, cost)`.
const CALLS: &[(&str, &str, u64, u64, f64)] = &[
    ("m-a", "t1", 1000, 0, 0.01),
    ("m-a", "t1", 1000, 800, 0.01),
    // After the first call in its thread and without a cache read: cold.
    ("m-a", "t1", 1000, 0, 0.01),
    ("m-b", "t2", 500, 0, 0.02),
];

/// Total spend of [`CALLS`], above the budget cap below.
const BUDGET_CAP_USD: f64 = 0.04;

fn peel(value: &Value) -> &Value {
    value.get("result").unwrap_or(value)
}

fn approx(value: &Value, want: f64) {
    let got = value.as_f64().expect("a number");
    assert!((got - want).abs() < 1e-9, "got {got}, want {want}");
}

#[tokio::test]
async fn cost_reports_group_the_ledger_and_a_refuse_budget_stops_the_call() {
    let workspace = tempfile::tempdir().expect("workspace");
    std::fs::write(
        workspace.path().join("config.toml"),
        format!(
            "[scheduler_gate]\nmode = \"always_on\"\n\n[cost]\nenabled = true\n\n\
             [[cost.budgets]]\nname = \"tiny cap\"\nscope = \"global\"\nperiod = \"month\"\n\
             max_usd = {BUDGET_CAP_USD}\naction = \"refuse\"\n"
        ),
    )
    .expect("seed config.toml");
    // This target runs one test in its own process; nothing else reads these.
    std::env::set_var("OPENHUMAN_WORKSPACE", workspace.path());
    std::env::set_var("OPENHUMAN_DISABLE_CHANNEL_LISTENERS", "1");

    let config = Config::load_or_init().await.expect("config");
    assert_eq!(config.cost.budgets.len(), 1, "the budget must load");
    cost::init_global(config.cost.clone(), &config.workspace_dir);
    let tracker = cost::try_global().expect("this process owns the global tracker");

    // Seed the ledger, oldest first.
    // Milliseconds apart, so the ledger stays inside the current budget month
    // even at a month boundary.
    let month_start = Utc::now()
        .date_naive()
        .with_day(1)
        .expect("first of the month")
        .and_hms_opt(0, 0, 0)
        .expect("midnight")
        .and_utc();
    let start = Utc::now() - Duration::milliseconds(CALLS.len() as i64 + 1);
    for (i, (model, thread, input, cached, cost_usd)) in CALLS.iter().enumerate() {
        let mut usage = TokenUsage::new(*model, *input, 10, 0.0, 0.0);
        usage.cached_input_tokens = *cached;
        usage.cost_usd = *cost_usd;
        // Never before the budget month's start (UTC midnight on the 1st).
        usage.timestamp = (start + Duration::milliseconds(i as i64)).max(month_start);
        usage.scope.thread_id = Some((*thread).to_string());
        usage.scope.agent_id = Some("orchestrator".to_string());
        tracker.record_usage_unconditional(usage).expect("record");
    }

    // cost_report grouped by model.
    let report = invoke_method(
        DispatchCtx::process_default(),
        default_state(),
        "openhuman.cost_report",
        json!({ "days": 1, "groupBy": ["model"] }),
    )
    .await
    .expect("cost_report");
    let report = peel(&report);
    let totals = &report["totals"];
    assert_eq!(totals["calls"], 4);
    assert_eq!(totals["input_tokens"], 3500);
    assert_eq!(totals["cached_input_tokens"], 800);
    approx(&totals["cache_hit_ratio"], 800.0 / 3500.0);
    approx(&totals["cost_usd"], 0.05);
    let rows = report["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 2, "one row per model: {rows:?}");
    // Most expensive first: m-a (0.03) before m-b (0.02).
    assert_eq!(rows[0]["key"]["model"], "m-a");
    assert_eq!(rows[0]["calls"], 3);
    approx(&rows[0]["cache_hit_ratio"], 800.0 / 3000.0);
    assert_eq!(rows[1]["key"]["model"], "m-b");
    assert_eq!(rows[1]["calls"], 1);
    approx(&rows[1]["cache_hit_ratio"], 0.0);

    // A filter narrows the report; a misspelt one is refused.
    let filtered = invoke_method(
        DispatchCtx::process_default(),
        default_state(),
        "openhuman.cost_report",
        json!({ "days": 1, "filter": { "model": "m-b" } }),
    )
    .await
    .expect("filtered cost_report");
    assert_eq!(peel(&filtered)["totals"]["calls"], 1);
    invoke_method(
        DispatchCtx::process_default(),
        default_state(),
        "openhuman.cost_report",
        json!({ "days": 1, "filter": { "modle": "m-b" } }),
    )
    .await
    .expect_err("an unknown filter key must be refused");

    // cost_cache_report: overall hit ratio and the cold call.
    let cache = invoke_method(
        DispatchCtx::process_default(),
        default_state(),
        "openhuman.cost_cache_report",
        json!({ "days": 1 }),
    )
    .await
    .expect("cost_cache_report");
    let cache = peel(&cache);
    approx(&cache["cache_hit_ratio"], 800.0 / 3500.0);
    assert_eq!(cache["cold_calls"], 1, "{cache}");
    let calls = cache["calls"].as_array().expect("calls");
    assert_eq!(calls.len(), 4);
    assert_eq!(calls.iter().filter(|c| c["cold"] == true).count(), 1);
    let warm = calls
        .iter()
        .find(|c| c["cached_input_tokens"] == 800)
        .expect("the cached call");
    approx(&warm["cache_hit_ratio"], 0.8);

    // The tiny refuse cap is exceeded by the seeded spend: the next model call
    // is refused before it is made.
    let gate = OpenHumanBudgetGate::new(Arc::new(config));
    let estimate = CallEstimate::new("m-a", 100, 100).with_agent("orchestrator");
    match gate.acquire(&estimate).await {
        Ok(permit) => {
            drop(permit);
            panic!("a spend over the refuse cap must stop the model call");
        }
        Err(err) => {
            let message = err.to_string();
            assert!(message.contains("BUDGET_EXCEEDED"), "{message}");
            assert!(message.contains("tiny cap"), "{message}");
        }
    }
}
