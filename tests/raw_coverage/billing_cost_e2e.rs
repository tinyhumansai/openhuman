//! RPC-level e2e coverage for `openhuman.billing_*`, `openhuman.cost_*` and
//! `openhuman.dashboard_model_health`.
//!
//! Every case drives the real JSON-RPC router (`build_core_http_router`) over
//! HTTP, exactly as the desktop app does. The billing cases talk to an
//! in-process mock of the hosted payments API; the cost and dashboard cases are
//! entirely local — cost reads a seeded `costs.jsonl`, and model health is a
//! pure projection of `config.toml`.
//!
//! A module of the aggregated `raw_coverage_all` target, not a target of its
//! own. Run with:
//!   cargo test --test raw_coverage_all --features "$(bash scripts/ci/product-features.sh)" \
//!       -- billing_cost_e2e

#[path = "w4_shared/mod.rs"]
mod support;

use serde_json::{json, Value};
use support::{assert_error, assert_no_error, error_message, logs, mock_log, peel, Harness};

/// Every persisted cost record the seeding helper writes, as one JSONL line.
fn cost_record_line(id: &str, model: &str, cost_usd: f64, input: u64, output: u64) -> String {
    let record = json!({
        "id": id,
        "session_id": "w4-seeded-session",
        "usage": {
            "model": model,
            "input_tokens": input,
            "output_tokens": output,
            "total_tokens": input + output,
            "cached_input_tokens": 0,
            "cache_creation_tokens": 0,
            "reasoning_tokens": 0,
            "cost_usd": cost_usd,
            "cost_source": "estimated",
            // Today, so it lands in the current day, the current month, and the
            // trailing-7-day dashboard window all at once.
            "timestamp": chrono::Utc::now().to_rfc3339(),
        }
    });
    format!("{record}\n")
}

// ── billing ──────────────────────────────────────────────────────────────────

/// The ten read/write billing controllers that had no e2e target, driven against
/// the mock payments backend with a stored session.
#[tokio::test]
async fn billing_uncovered_controllers_round_trip_against_the_backend() {
    let _lock = support::env_lock();
    let harness = Harness::start("", false).await;
    let log = mock_log();
    log.clear();
    harness.login().await;

    // --- billing_get_balance ------------------------------------------------
    let balance = harness
        .call(10, "openhuman.billing_get_balance", json!({}))
        .await;
    let balance = peel(assert_no_error(&balance, "billing_get_balance"));
    assert_eq!(
        balance.get("balanceUsd").and_then(Value::as_f64),
        Some(42.5),
        "the adapter must surface the backend's `data` payload verbatim: {balance}"
    );
    assert_eq!(
        balance.get("currency").and_then(Value::as_str),
        Some("USD"),
        "every field of `data` should survive, not just the first: {balance}"
    );

    // --- billing_get_transactions: defaults are part of the contract --------
    let txns = harness
        .call(11, "openhuman.billing_get_transactions", json!({}))
        .await;
    let txns_result = assert_no_error(&txns, "billing_get_transactions");
    let txns_inner = peel(txns_result);
    let rows = txns_inner
        .get("transactions")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("expected a `transactions` array: {txns_inner}"));
    assert_eq!(rows.len(), 2, "seeded two transactions: {txns_inner}");
    assert_eq!(
        rows[0].get("id").and_then(Value::as_str),
        Some("txn-1"),
        "order must be preserved through the adapter: {txns_inner}"
    );
    let recorded = log
        .first_with_path_prefix("/payments/credits/transactions")
        .expect("the transactions call must have reached the backend");
    assert_eq!(
        recorded.get("path").and_then(Value::as_str),
        Some("/payments/credits/transactions?limit=20&offset=0"),
        "an empty params map must still send the documented limit=20/offset=0 defaults: {recorded}"
    );

    // ...and an explicit page is forwarded as given.
    let paged = harness
        .call(
            12,
            "openhuman.billing_get_transactions",
            json!({ "limit": 5, "offset": 10 }),
        )
        .await;
    assert_no_error(&paged, "billing_get_transactions paged");
    let paths: Vec<String> = log
        .entries()
        .iter()
        .filter_map(|e| e.get("path").and_then(Value::as_str).map(str::to_string))
        .collect();
    assert!(
        paths.contains(&"/payments/credits/transactions?limit=5&offset=10".to_string()),
        "explicit limit/offset must reach the backend as a query string; saw {paths:?}"
    );

    // --- billing_get_auto_recharge -----------------------------------------
    let auto = harness
        .call(13, "openhuman.billing_get_auto_recharge", json!({}))
        .await;
    let auto = peel(assert_no_error(&auto, "billing_get_auto_recharge"));
    assert_eq!(auto.get("enabled").and_then(Value::as_bool), Some(true));
    assert_eq!(
        auto.get("thresholdUsd").and_then(Value::as_f64),
        Some(5.0),
        "auto-recharge settings must round-trip unchanged: {auto}"
    );

    // --- billing_update_auto_recharge: the payload is forwarded verbatim ----
    let updated = harness
        .call(
            14,
            "openhuman.billing_update_auto_recharge",
            json!({ "payload": { "enabled": false, "thresholdUsd": 2.0 } }),
        )
        .await;
    let updated = peel(assert_no_error(&updated, "billing_update_auto_recharge"));
    assert_eq!(
        updated.get("enabled").and_then(Value::as_bool),
        Some(false),
        "the mock echoes the PATCH body; a reshaped payload would show here: {updated}"
    );
    assert_eq!(updated.get("updated").and_then(Value::as_bool), Some(true));
    let patch = log
        .first_with_path_prefix("/payments/credits/auto-recharge")
        .expect("no auto-recharge request reached the backend");
    assert!(
        log.entries().iter().any(|e| {
            e.get("method").and_then(Value::as_str) == Some("PATCH")
                && e.get("body").and_then(|b| b.get("thresholdUsd")).and_then(Value::as_f64)
                    == Some(2.0)
        }),
        "the PATCH body must carry the caller's payload unwrapped (no `payload` envelope); saw {:?}",
        patch
    );

    // --- billing_get_cards --------------------------------------------------
    let cards = harness
        .call(15, "openhuman.billing_get_cards", json!({}))
        .await;
    let cards = peel(assert_no_error(&cards, "billing_get_cards"));
    let cards_arr = cards
        .as_array()
        .unwrap_or_else(|| panic!("expected an array of cards: {cards}"));
    assert_eq!(cards_arr.len(), 2, "seeded two saved cards: {cards}");
    assert_eq!(
        cards_arr[0].get("last4").and_then(Value::as_str),
        Some("4242")
    );

    // --- billing_create_setup_intent ---------------------------------------
    let intent = harness
        .call(16, "openhuman.billing_create_setup_intent", json!({}))
        .await;
    let intent = peel(assert_no_error(&intent, "billing_create_setup_intent"));
    assert_eq!(
        intent.get("clientSecret").and_then(Value::as_str),
        Some("seti_w4_secret"),
        "a SetupIntent with no clientSecret is useless to the frontend: {intent}"
    );

    // --- billing_update_card: the id is percent-encoded into the path -------
    // `pm/with space` cannot escape its path segment; the mock decodes it back,
    // so an unencoded id would 404 here instead of echoing.
    let card_update = harness
        .call(
            17,
            "openhuman.billing_update_card",
            json!({ "paymentMethodId": "pm/with space", "payload": { "isDefault": true } }),
        )
        .await;
    let card_update = peel(assert_no_error(&card_update, "billing_update_card"));
    assert_eq!(
        card_update.get("paymentMethodId").and_then(Value::as_str),
        Some("pm/with space"),
        "the id must survive percent-encoding into the path and decode back: {card_update}"
    );
    assert_eq!(
        card_update.get("isDefault").and_then(Value::as_bool),
        Some(true),
        "the PATCH payload must reach the backend: {card_update}"
    );

    // --- billing_delete_card ------------------------------------------------
    let deleted = harness
        .call(
            18,
            "openhuman.billing_delete_card",
            json!({ "paymentMethodId": "pm_2" }),
        )
        .await;
    let deleted = peel(assert_no_error(&deleted, "billing_delete_card"));
    assert_eq!(
        deleted.get("deleted").and_then(Value::as_str),
        Some("pm_2"),
        "delete must name the card it removed: {deleted}"
    );

    // --- billing_redeem_coupon (happy) -------------------------------------
    let redeemed = harness
        .call(
            19,
            "openhuman.billing_redeem_coupon",
            json!({ "code": "WELCOME10" }),
        )
        .await;
    let redeemed_outer = assert_no_error(&redeemed, "billing_redeem_coupon");
    assert!(
        logs(redeemed_outer).iter().any(|l| l == "coupon redeemed"),
        "the outcome must carry its log line: {redeemed_outer}"
    );
    let redeemed = peel(redeemed_outer);
    assert_eq!(
        redeemed.get("creditsUsd").and_then(Value::as_f64),
        Some(10.0),
        "the redemption result must reach the caller: {redeemed}"
    );

    // --- billing_get_coupons ------------------------------------------------
    let coupons = harness
        .call(20, "openhuman.billing_get_coupons", json!({}))
        .await;
    let coupons = peel(assert_no_error(&coupons, "billing_get_coupons"));
    let coupons_arr = coupons
        .as_array()
        .unwrap_or_else(|| panic!("expected an array of coupons: {coupons}"));
    assert_eq!(coupons_arr.len(), 1, "seeded one redeemed coupon: {coupons}");
    assert_eq!(
        coupons_arr[0].get("code").and_then(Value::as_str),
        Some("WELCOME10")
    );
}

/// The failure half: backend rejections must surface, and the pre-HTTP
/// validation in `billing/ops.rs` must reject before any request is made.
#[tokio::test]
async fn billing_rejects_bad_input_before_it_reaches_the_backend() {
    let _lock = support::env_lock();
    let harness = Harness::start("", false).await;
    harness.login().await;
    let log = mock_log();

    // A coupon the backend refuses — the 400 must reach the caller, not be
    // swallowed into an empty success.
    let bad_coupon = harness
        .call(
            30,
            "openhuman.billing_redeem_coupon",
            json!({ "code": "NOPE" }),
        )
        .await;
    let message = error_message(&bad_coupon, "billing_redeem_coupon unknown code");
    assert!(
        message.contains("not redeemable"),
        "the backend's rejection text must survive to the caller; got: {message}"
    );

    // Everything below must fail *without* a network call. Clearing the log
    // first is what makes the "no request was made" assertion meaningful.
    log.clear();

    let blank_code = harness
        .call(31, "openhuman.billing_redeem_coupon", json!({ "code": "  " }))
        .await;
    assert!(
        error_message(&blank_code, "blank coupon").contains("code is required"),
        "a whitespace-only coupon code must be rejected locally: {blank_code}"
    );

    let blank_card = harness
        .call(
            32,
            "openhuman.billing_delete_card",
            json!({ "paymentMethodId": "   " }),
        )
        .await;
    assert!(
        error_message(&blank_card, "blank paymentMethodId").contains("paymentMethodId is required"),
        "a blank card id must never be encoded into a DELETE path: {blank_card}"
    );

    assert!(
        log.entries().is_empty(),
        "input validation is documented as pre-HTTP, but these requests reached the backend: {:?}",
        log.entries()
    );
}

/// With no stored session the adapters must refuse locally rather than firing a
/// doomed unauthenticated request.
#[tokio::test]
async fn billing_without_a_session_refuses_locally() {
    let _lock = support::env_lock();
    let harness = Harness::start("", false).await;
    let log = mock_log();
    log.clear();

    let balance = harness
        .call(40, "openhuman.billing_get_balance", json!({}))
        .await;
    let message = error_message(&balance, "billing_get_balance with no session");
    assert!(
        message.contains("no backend session token")
            && message.contains("auth_store_session"),
        "the error must name the missing session *and* the call that fixes it, \
         so the UI can route to sign-in rather than show a bare failure; got: {message}"
    );

    let cards = harness
        .call(41, "openhuman.billing_get_cards", json!({}))
        .await;
    assert_error(&cards, "billing_get_cards with no session");

    assert!(
        !log.entries().iter().any(|e| e
            .get("path")
            .and_then(Value::as_str)
            .is_some_and(|p| p.starts_with("/payments"))),
        "a session-less billing call must not hit /payments at all: {:?}",
        log.entries()
    );
}

// ── cost ─────────────────────────────────────────────────────────────────────

/// All four `cost_*` controllers over a seeded `costs.jsonl`.
///
/// The seed deliberately mixes a **managed** tier slug with a **BYOK** model id:
/// #5016 makes the monthly budget count managed spend only, so `period_total_usd`
/// and `month_to_date_usd` must disagree. A dashboard that summed everything
/// into the budget would pass a "not empty" test and fail this one.
#[tokio::test]
async fn cost_controllers_report_seeded_usage_and_split_managed_from_byok() {
    let _lock = support::env_lock();
    let harness = Harness::start(
        r#"
[cost]
enabled = true
monthly_limit_usd = 10.0

[cost.dashboard]
enabled = true
currency = "USD"
warn_threshold = 0.5
alert_threshold = 0.9
"#,
        true,
    )
    .await;

    // Seed the append-only log the tracker reads. `chat-v1` is a managed tier
    // slug; `anthropic/claude-sonnet-4-20250514` is BYOK.
    let state_dir = harness.workspace().join("state");
    std::fs::create_dir_all(&state_dir).expect("create state dir");
    let mut jsonl = String::new();
    jsonl.push_str(&cost_record_line("rec-managed", "chat-v1", 6.0, 1_000, 500));
    jsonl.push_str(&cost_record_line(
        "rec-byok",
        "anthropic/claude-sonnet-4-20250514",
        4.0,
        2_000,
        800,
    ));
    jsonl.push_str(&cost_record_line(
        "rec-embed",
        "openai/text-embedding-3-small",
        0.5,
        300,
        0,
    ));
    std::fs::write(state_dir.join("costs.jsonl"), &jsonl).expect("seed costs.jsonl");

    // --- cost_get_summary ---------------------------------------------------
    let summary = harness
        .call(50, "openhuman.cost_get_summary", json!({}))
        .await;
    let summary = peel(assert_no_error(&summary, "cost_get_summary"));
    assert_eq!(
        summary.get("daily_cost_usd").and_then(Value::as_f64),
        Some(10.5),
        "today's total must be every seeded record, managed and BYOK alike: {summary}"
    );
    assert_eq!(
        summary.get("monthly_cost_usd").and_then(Value::as_f64),
        Some(10.5),
        "the seeded records are all in the current month: {summary}"
    );
    assert_eq!(
        summary.get("session_cost_usd").and_then(Value::as_f64),
        Some(0.0),
        "the RPC fallback tracker has recorded nothing itself, so the *session* \
         figure must stay zero even though the file is full: {summary}"
    );

    // --- cost_get_dashboard -------------------------------------------------
    let dashboard = harness
        .call(51, "openhuman.cost_get_dashboard", json!({}))
        .await;
    let dashboard = peel(assert_no_error(&dashboard, "cost_get_dashboard"));
    assert_eq!(
        dashboard.get("period_total_usd").and_then(Value::as_f64),
        Some(10.5),
        "the 7-day period total counts all routes: {dashboard}"
    );
    assert_eq!(
        dashboard.get("month_to_date_usd").and_then(Value::as_f64),
        Some(10.5),
        "month-to-date is the all-routes figure — the split lives in the budget \
         fields below, not here: {dashboard}"
    );
    assert_eq!(
        dashboard.get("budget_utilization").and_then(Value::as_f64),
        Some(0.6),
        "#5016: the gauge is driven by *managed* spend only — 6.0 of `chat-v1` \
         against a 10.0 monthly limit is 60%. Driving it off the 10.5 all-routes \
         total would read 100% against a cap BYOK spend can never trip: {dashboard}"
    );
    assert_eq!(
        dashboard.get("budget_status").and_then(Value::as_str),
        Some("warning"),
        "60% managed utilisation is past the configured 0.5 warn threshold and \
         short of 0.9; the all-routes 105% would have read `exceeded`: {dashboard}"
    );
    assert_eq!(
        dashboard.get("currency").and_then(Value::as_str),
        Some("USD")
    );
    let days = dashboard
        .get("days")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("dashboard must carry a `days` array: {dashboard}"));
    assert_eq!(
        days.len(),
        7,
        "the dashboard window is 7 days, zero-filled: {dashboard}"
    );
    let today = days.last().expect("7 entries");
    assert_eq!(
        today.get("cost_usd").and_then(Value::as_f64),
        Some(10.5),
        "the newest day is last and holds every seeded record: {today}"
    );
    let by_model = dashboard
        .get("by_model")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("dashboard must carry `by_model`: {dashboard}"));
    assert_eq!(by_model.len(), 3, "three distinct models seeded: {by_model:?}");
    let managed = by_model
        .iter()
        .find(|m| m.get("model").and_then(Value::as_str) == Some("chat-v1"))
        .unwrap_or_else(|| panic!("`chat-v1` missing from by_model: {by_model:?}"));
    assert!(
        managed.get("provider").is_some_and(Value::is_null),
        "a bare tier slug has no `provider/` prefix, so provider stays null \
         rather than being invented: {managed}"
    );
    let byok = by_model
        .iter()
        .find(|m| {
            m.get("model").and_then(Value::as_str) == Some("anthropic/claude-sonnet-4-20250514")
        })
        .expect("BYOK model missing from by_model");
    assert_eq!(
        byok.get("provider").and_then(Value::as_str),
        Some("anthropic"),
        "provider is derived from the slash prefix: {byok}"
    );

    // --- cost_get_daily_history --------------------------------------------
    let history = harness
        .call(
            52,
            "openhuman.cost_get_daily_history",
            json!({ "days": 3 }),
        )
        .await;
    let history = peel(assert_no_error(&history, "cost_get_daily_history"));
    let entries = history
        .as_array()
        .unwrap_or_else(|| panic!("daily history must be an array: {history}"));
    assert_eq!(entries.len(), 3, "asked for 3 days: {history}");
    assert_eq!(
        entries[0].get("cost_usd").and_then(Value::as_f64),
        Some(0.0),
        "gap days must be zero-filled, not omitted: {history}"
    );
    assert_eq!(
        entries[2].get("cost_usd").and_then(Value::as_f64),
        Some(10.5),
        "oldest first, so today is last: {history}"
    );
    assert_eq!(
        entries[2].get("request_count").and_then(Value::as_u64),
        Some(3),
        "three records landed today: {history}"
    );

    // An out-of-range span is clamped rather than rejected.
    let clamped = harness
        .call(
            53,
            "openhuman.cost_get_daily_history",
            json!({ "days": 100_000 }),
        )
        .await;
    let clamped = peel(assert_no_error(&clamped, "cost_get_daily_history clamped"));
    assert_eq!(
        clamped.as_array().map(Vec::len),
        Some(366),
        "days is documented as clamped to [1, 366]: {clamped}"
    );

    // --- cost_get_usage_log -------------------------------------------------
    let usage = harness
        .call(54, "openhuman.cost_get_usage_log", json!({}))
        .await;
    let usage = peel(assert_no_error(&usage, "cost_get_usage_log"));
    assert_eq!(
        usage.get("request_count").and_then(Value::as_u64),
        Some(3),
        "every seeded record is inside the default 30-day window: {usage}"
    );
    assert_eq!(
        usage.get("total_cost_usd").and_then(Value::as_f64),
        Some(10.5)
    );
    assert_eq!(
        usage.get("days").and_then(Value::as_u64),
        Some(30),
        "the default span is echoed back: {usage}"
    );
    assert_eq!(
        usage.get("limit").and_then(Value::as_u64),
        Some(250),
        "the default limit is echoed back: {usage}"
    );
    let categories: Vec<(&str, f64)> = usage
        .get("by_category")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("usage log must carry `by_category`: {usage}"))
        .iter()
        .map(|c| {
            (
                c.get("category").and_then(Value::as_str).unwrap_or(""),
                c.get("cost_usd").and_then(Value::as_f64).unwrap_or(f64::NAN),
            )
        })
        .collect();
    assert_eq!(
        categories,
        vec![("AI chat and reasoning", 10.0), ("Embeddings", 0.5)],
        "categories are inferred from the model id and sorted by spend: {usage}"
    );
    let records = usage
        .get("records")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("usage log must carry `records`: {usage}"));
    assert_eq!(records.len(), 3);
    assert!(
        records.iter().all(|r| r.get("cost_source").and_then(Value::as_str)
            == Some("estimated")),
        "persisted cost provenance must survive into the DTO: {records:?}"
    );

    // An over-large limit is clamped, and the clamped value is what is reported.
    let capped = harness
        .call(
            55,
            "openhuman.cost_get_usage_log",
            json!({ "days": 1, "limit": 99_999 }),
        )
        .await;
    let capped = peel(assert_no_error(&capped, "cost_get_usage_log clamped"));
    assert_eq!(
        capped.get("limit").and_then(Value::as_u64),
        Some(1000),
        "limit is documented as clamped to [1, 1000]: {capped}"
    );
    assert_eq!(capped.get("days").and_then(Value::as_u64), Some(1));
}

/// The empty-workspace path: no `costs.jsonl` at all must still answer, not error.
#[tokio::test]
async fn cost_controllers_answer_on_a_workspace_with_no_history() {
    let _lock = support::env_lock();
    let harness = Harness::start("", true).await;

    let summary = harness
        .call(60, "openhuman.cost_get_summary", json!({}))
        .await;
    let summary = peel(assert_no_error(&summary, "cost_get_summary empty"));
    assert_eq!(
        summary.get("daily_cost_usd").and_then(Value::as_f64),
        Some(0.0),
        "a fresh workspace has zero spend, not an error: {summary}"
    );
    assert_eq!(
        summary.get("request_count").and_then(Value::as_u64),
        Some(0)
    );

    let usage = harness
        .call(61, "openhuman.cost_get_usage_log", json!({}))
        .await;
    let usage = peel(assert_no_error(&usage, "cost_get_usage_log empty"));
    assert_eq!(
        usage.get("records").and_then(Value::as_array).map(Vec::len),
        Some(0),
        "no records, but the envelope must still be well formed: {usage}"
    );
    assert_eq!(
        usage.get("by_category").and_then(Value::as_array).map(Vec::len),
        Some(0)
    );
}

// ── dashboard ────────────────────────────────────────────────────────────────

/// `dashboard_model_health` projects `model_registry` and the configured
/// thresholds, with telemetry fields held as documented placeholders.
#[tokio::test]
async fn dashboard_model_health_projects_the_registry_and_thresholds() {
    let _lock = support::env_lock();
    let harness = Harness::start(
        r#"
[dashboard.model_health]
enabled = true
hallucination_threshold = 0.11
min_tasks_for_rating = 7
evaluation_window_tasks = 42

[[model_registry]]
id = "w4/alpha"
provider = "w4"
cost_per_1m_input = 1.5
cost_per_1m_output = 6.0
context_window = 200000
vision = true

[[model_registry]]
id = "w4/beta"
provider = "w4"
cost_per_1m_input = 0.25
cost_per_1m_output = 1.0
context_window = 32000
vision = false
"#,
        true,
    )
    .await;

    let health = harness
        .call(70, "openhuman.dashboard_model_health", json!({}))
        .await;
    let health_outer = assert_no_error(&health, "dashboard_model_health");
    assert!(
        logs(health_outer)
            .iter()
            .any(|l| l.contains("returned 2 models")),
        "the log line must report the row count it actually built: {health_outer}"
    );
    let health = peel(health_outer);

    let models = health
        .get("models")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("expected a `models` array: {health}"));
    assert_eq!(
        models.len(),
        2,
        "one row per registry entry, in registry order: {health}"
    );
    assert_eq!(models[0].get("id").and_then(Value::as_str), Some("w4/alpha"));
    assert_eq!(
        models[0].get("cost_per_1m_output").and_then(Value::as_f64),
        Some(6.0),
        "cost must come from the registry, not a default: {health}"
    );
    assert_eq!(
        models[0].get("context_window").and_then(Value::as_u64),
        Some(200_000)
    );
    assert_eq!(models[0].get("vision").and_then(Value::as_bool), Some(true));
    assert_eq!(models[1].get("vision").and_then(Value::as_bool), Some(false));

    // The placeholder contract, which the frontend reads as "no signal".
    for row in models {
        assert!(
            row.get("quality_score").is_some_and(Value::is_null),
            "quality_score is a documented `null` placeholder until telemetry lands: {row}"
        );
        assert!(
            row.get("hallucination_rate").is_some_and(Value::is_null),
            "hallucination_rate is a documented `null` placeholder: {row}"
        );
        assert_eq!(
            row.get("agents_using").and_then(Value::as_u64),
            Some(0),
            "`agents_using` is a documented `0` placeholder — there is no local \
             telemetry sink to populate it yet: {row}"
        );
        assert_eq!(
            row.get("tasks_evaluated").and_then(Value::as_u64),
            Some(0),
            "`tasks_evaluated` is a documented `0` placeholder: {row}"
        );
    }

    let cfg = health
        .get("config")
        .unwrap_or_else(|| panic!("expected a `config` view: {health}"));
    assert_eq!(
        cfg.get("hallucination_threshold").and_then(Value::as_f64),
        Some(0.11),
        "thresholds must come from config so the frontend can badge rows: {cfg}"
    );
    assert_eq!(
        cfg.get("min_tasks_for_rating").and_then(Value::as_u64),
        Some(7)
    );
    assert_eq!(
        cfg.get("evaluation_window_tasks").and_then(Value::as_u64),
        Some(42)
    );
}

/// The disabled-feature path is an error, not an empty table.
#[tokio::test]
async fn dashboard_model_health_refuses_when_disabled() {
    let _lock = support::env_lock();
    let harness = Harness::start(
        r#"
[dashboard.model_health]
enabled = false
"#,
        true,
    )
    .await;

    let health = harness
        .call(80, "openhuman.dashboard_model_health", json!({}))
        .await;
    let message = error_message(&health, "dashboard_model_health disabled");
    assert!(
        message.contains("model health disabled"),
        "a disabled panel must say so rather than return an empty list; got: {message}"
    );
}
