use super::*;
use std::time::Duration;

fn gate(compression: AgentTokenjuiceCompression) -> OpenHumanBudgetGate {
    OpenHumanBudgetGate::with_compression(Arc::new(Config::default()), compression)
}

fn crowded() -> ContextState {
    ContextState {
        message_count: 40,
        prompt_tokens: 95_000,
        context_window_tokens: Some(100_000),
        iterations: 12,
    }
}

#[test]
fn seeds_the_attributed_model_from_config() {
    let mut config = Config::default();
    config.default_model = Some("pinned-model".into());
    let gate = OpenHumanBudgetGate::new(Arc::new(config));
    assert_eq!(gate.attributed_model(), Some("pinned-model".to_string()));

    let mut unpinned = Config::default();
    unpinned.default_model = None;
    let gate = OpenHumanBudgetGate::new(Arc::new(unpinned));
    assert_eq!(gate.attributed_model(), None);
}

#[tokio::test]
async fn acquire_stamps_the_model_that_record_will_attribute() {
    // Mismatch (1) in the module docs: `Usage` has no model, so `acquire`
    // is the only place the attribution can come from.
    let gate = gate(AgentTokenjuiceCompression::Auto);
    gate.acquire(&CallEstimate::new("some/model", 10, 5))
        .await
        .expect("no tracker in tests, so nothing can refuse");
    assert_eq!(gate.attributed_model(), Some("some/model".to_string()));
}

#[tokio::test]
async fn an_empty_model_does_not_erase_the_attribution() {
    let mut config = Config::default();
    config.default_model = Some("pinned-model".into());
    let gate = OpenHumanBudgetGate::new(Arc::new(config));
    gate.acquire(&CallEstimate::new("   ", 1, 1))
        .await
        .expect("grants");
    assert_eq!(gate.attributed_model(), Some("pinned-model".to_string()));
}

#[tokio::test]
async fn the_permit_reserves_the_estimated_total() {
    let gate = gate(AgentTokenjuiceCompression::Auto);
    let permit = gate
        .acquire(&CallEstimate::new("m", 100, 20))
        .await
        .expect("grants");
    assert_eq!(permit.reserved_tokens(), Some(120));
    assert!(permit.id().is_some(), "grants are correlatable");
}

/// An interactive gate must never enter the background scheduler.
///
/// The gate's `Paused` arm polls until background AI is re-enabled, so a
/// user-initiated turn that queued there would hang until the turn timeout
/// for anyone signed out on a local/BYOK model, or who merely paused
/// background AI. The timeout turns that stall into a failure rather than a
/// hung test.
#[tokio::test]
async fn an_interactive_gate_does_not_queue_behind_the_background_scheduler() {
    let gate = gate(AgentTokenjuiceCompression::Auto);
    assert!(!gate.background, "interactive is the default");

    let permit = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        gate.acquire(&CallEstimate::new("m", 100, 20)),
    )
    .await
    .expect("an interactive acquire must not wait on the background gate")
    .expect("grants");

    assert!(permit.id().is_some(), "grants stay correlatable");
    assert_eq!(permit.reserved_tokens(), Some(120));
}

#[tokio::test]
async fn dropping_the_crate_permit_releases_the_scheduler_permit() {
    // The global LLM semaphore has a single slot, so a leaked `LlmPermit`
    // makes the second acquire hang forever. The timeout turns that leak
    // into a failure instead of a hung test — this is the regression this
    // whole adapter is most likely to break.
    //
    // Must be a *background* gate: an interactive one skips the scheduler
    // entirely, so this would pass without ever exercising the release the
    // test exists to prove.
    let gate = gate(AgentTokenjuiceCompression::Auto).as_background_work();
    for round in 0..3 {
        let permit = tokio::time::timeout(
            Duration::from_secs(5),
            gate.acquire(&CallEstimate::new("m", 1, 1)),
        )
        .await
        .unwrap_or_else(|_| panic!("round {round} blocked: the previous permit leaked"))
        .expect("grants");
        drop(permit);
    }
}

#[tokio::test]
async fn explicit_release_returns_capacity_before_end_of_scope() {
    // Background, for the same reason as the test above.
    let gate = gate(AgentTokenjuiceCompression::Auto).as_background_work();
    let first = gate
        .acquire(&CallEstimate::new("m", 1, 1))
        .await
        .expect("grants");
    first.release();
    tokio::time::timeout(
        Duration::from_secs(5),
        gate.acquire(&CallEstimate::new("m", 1, 1)),
    )
    .await
    .expect("release freed the slot immediately")
    .expect("grants");
}

#[tokio::test]
async fn recording_usage_does_not_write_a_second_ledger_row() {
    // The event bridge already records every model call under its real model.
    // A `host:<agent>` row from this gate doubled tokens and request counts.
    const MODEL: &str = "budget-gate-probe/no-duplicate-row";
    let tmp = tempfile::TempDir::new().unwrap();
    let mut cost_config = crate::config::CostConfig::default();
    cost_config.enabled = true;
    crate::platform::cost::rebind_global(cost_config, tmp.path());

    let gate = gate(AgentTokenjuiceCompression::Auto);
    let _permit = gate
        .acquire(&CallEstimate::new(MODEL, 1_000, 250))
        .await
        .expect("grants");
    gate.record(&Usage::new(1_000, 250))
        .await
        .expect("recording never fails the turn");

    let ledger =
        std::fs::read_to_string(tmp.path().join("state").join("costs.jsonl")).unwrap_or_default();
    assert!(
        !ledger.contains(MODEL) && !ledger.contains("host:"),
        "the gate must not write to the cost ledger, got: {ledger}"
    );
}

#[tokio::test]
async fn recording_usage_is_a_soft_no_op_without_a_tracker() {
    // `cost::try_global()` is `None` in unit tests. Recording must still
    // succeed — the trait says `record` is called even for failed calls,
    // and a metering hiccup must never fail a turn.
    let gate = gate(AgentTokenjuiceCompression::Auto);
    gate.record(&Usage::new(1_000, 250))
        .await
        .expect("recording never fails the turn");
}

#[test]
fn this_gate_never_asks_for_compression() {
    // Union semantics: `None` is "not asking", not "do not compress". This
    // gate has no budget opinion left to escalate from — the spend cap that
    // used to drive the hint is gone — and context fullness was never its
    // question, so even a full window yields `None` and the crate's own
    // SummarizationPolicy stays the authority.
    let gate = gate(AgentTokenjuiceCompression::Full);
    assert_eq!(gate.compression_hint(&crowded()), CompressionHint::None);
}

fn ledger_with(spent_usd: f64, agent: &str) -> (tempfile::TempDir, cost::CostTracker) {
    let tmp = tempfile::tempdir().unwrap();
    let tracker = cost::CostTracker::new(crate::config::CostConfig::default(), tmp.path()).unwrap();
    let mut usage = cost::TokenUsage::new("m", 100, 50, 0.0, 0.0);
    usage.cost_usd = spent_usd;
    usage.scope = cost::UsageScope {
        agent_id: Some(agent.into()),
        ..Default::default()
    };
    tracker.record_usage_unconditional(usage).unwrap();
    (tmp, tracker)
}

/// A config whose `config.toml` does not exist, so the gate keeps the
/// in-memory budgets instead of reading the developer's real file.
fn config_without_file() -> Config {
    Config {
        config_path: std::env::temp_dir()
            .join(format!("oh-budget-gate-{}", uuid::Uuid::new_v4()))
            .join("config.toml"),
        ..Config::default()
    }
}

fn budgeted_gate(action: crate::config::BudgetAction) -> OpenHumanBudgetGate {
    let mut config = config_without_file();
    config.cost.budgets = vec![crate::config::BudgetPolicy {
        name: Some("planner cap".into()),
        scope: crate::config::BudgetScope::Agent,
        matches: Some("planner".into()),
        period: crate::config::BudgetPeriod::Month,
        max_usd: Some(1.0),
        max_tokens: None,
        warn_fraction: 0.8,
        action,
    }];
    OpenHumanBudgetGate::new(Arc::new(config))
}

fn estimate_for(agent: &str) -> CallEstimate {
    CallEstimate {
        model: "m".into(),
        agent_id: Some(agent.into()),
        ..CallEstimate::default()
    }
}

#[test]
fn a_refusing_budget_refuses_the_agent_over_it() {
    let (_tmp, tracker) = ledger_with(2.0, "planner");
    let gate = budgeted_gate(crate::config::BudgetAction::Refuse);
    let refusal = gate
        .check_budgets_against(&estimate_for("planner"), &gate.live_budgets(), &tracker)
        .expect("planner is over its cap");
    assert!(refusal.starts_with("BUDGET_EXCEEDED:"), "{refusal}");
    assert!(refusal.contains("planner cap"), "{refusal}");
    assert!(
        gate.check_budgets_against(
            &estimate_for("orchestrator"),
            &gate.live_budgets(),
            &tracker
        )
        .is_none(),
        "the cap matches only the planner"
    );
}

#[test]
fn a_warning_budget_never_refuses() {
    let (_tmp, tracker) = ledger_with(2.0, "planner");
    let gate = budgeted_gate(crate::config::BudgetAction::Warn);
    assert!(gate
        .check_budgets_against(&estimate_for("planner"), &gate.live_budgets(), &tracker)
        .is_none());
}

#[tokio::test]
async fn without_budgets_acquire_is_unchanged() {
    let gate = gate(AgentTokenjuiceCompression::Auto);
    assert!(gate.check_budgets(&estimate_for("planner")).is_none());
    assert!(gate.acquire(&estimate_for("planner")).await.is_ok());
}

#[test]
fn budgets_are_re_read_from_the_session_config_file() {
    // Built with no budgets; the file on disk now holds a refusing one.
    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    std::fs::write(
        &config_path,
        r#"
[[cost.budgets]]
name = "live cap"
scope = "agent"
match = "planner"
max_usd = 1.0
action = "refuse"
"#,
    )
    .unwrap();
    let config = Config {
        config_path,
        ..Config::default()
    };
    let gate = OpenHumanBudgetGate::new(Arc::new(config));
    let policies = gate.live_budgets();
    assert_eq!(policies.len(), 1);
    let (_ledger, tracker) = ledger_with(2.0, "planner");
    let refusal = gate
        .check_budgets_against(&estimate_for("planner"), &policies, &tracker)
        .expect("the live budget refuses");
    assert!(refusal.contains("live cap"), "{refusal}");
}

#[test]
fn a_missing_config_file_keeps_the_session_budgets() {
    let gate = budgeted_gate(crate::config::BudgetAction::Refuse);
    assert_eq!(gate.live_budgets().len(), 1);
}

#[test]
fn a_call_is_checked_against_its_own_model() {
    let (_tmp, tracker) = ledger_with(2.0, "planner");
    let mut config = config_without_file();
    config.cost.budgets = vec![crate::config::BudgetPolicy {
        name: Some("model cap".into()),
        scope: crate::config::BudgetScope::Model,
        matches: Some("m".into()),
        period: crate::config::BudgetPeriod::Month,
        max_usd: Some(1.0),
        max_tokens: None,
        warn_fraction: 0.8,
        action: crate::config::BudgetAction::Refuse,
    }];
    let gate = OpenHumanBudgetGate::new(Arc::new(config));
    // Another call left a different model in the shared attribution.
    *gate.last_model.write() = Some("other".into());
    let policies = gate.live_budgets();
    assert!(gate
        .check_budgets_against(&estimate_for("planner"), &policies, &tracker)
        .is_some());
}

#[tokio::test]
async fn acquire_refuses_an_over_budget_call() {
    let (_tmp, tracker) = ledger_with(2.0, "planner");
    let gate = budgeted_gate(crate::config::BudgetAction::Refuse).with_tracker(Arc::new(tracker));
    let err = match gate.acquire(&estimate_for("planner")).await {
        Err(err) => err,
        Ok(_) => panic!("an over-budget call must be refused"),
    };
    match err {
        tinyagents_harness::error::TinyAgentsError::LimitExceeded(message) => {
            assert!(message.starts_with("BUDGET_EXCEEDED:"), "{message}");
        }
        other => panic!("expected LimitExceeded, got {other:?}"),
    }
    // The same gate admits an agent the budget does not cover.
    assert!(gate.acquire(&estimate_for("orchestrator")).await.is_ok());
}

/// With no `default_model` and an estimate that names none, the call still
/// counts against budgets that don't depend on the model; skipping them let
/// every configured cap through.
#[test]
fn a_call_with_no_model_is_still_checked_against_its_budgets() {
    let (_tmp, tracker) = ledger_with(2.0, "planner");
    let gate = budgeted_gate(crate::config::BudgetAction::Refuse);
    let gate = OpenHumanBudgetGate::new(Arc::new(Config {
        default_model: None,
        ..(*gate.config).clone()
    }));
    let estimate = CallEstimate {
        model: String::new(),
        ..estimate_for("planner")
    };
    let refusal = gate
        .check_budgets_against(&estimate, &gate.live_budgets(), &tracker)
        .expect("planner is over its cap whatever the model");
    assert!(refusal.starts_with("BUDGET_EXCEEDED:"), "{refusal}");
}

/// ...but a model-scoped budget, which has no model to match, does not count it.
#[test]
fn a_call_with_no_model_is_outside_model_scoped_budgets() {
    let (_tmp, tracker) = ledger_with(0.0, "planner");
    let mut config = config_without_file();
    config.default_model = None;
    config.cost.budgets = vec![crate::config::BudgetPolicy {
        name: Some("per-model token cap".into()),
        scope: crate::config::BudgetScope::Model,
        matches: None,
        period: crate::config::BudgetPeriod::Month,
        max_usd: None,
        max_tokens: Some(1),
        warn_fraction: 0.8,
        action: crate::config::BudgetAction::Refuse,
    }];
    let gate = OpenHumanBudgetGate::new(Arc::new(config));
    let estimate = CallEstimate {
        model: String::new(),
        estimated_input_tokens: 10,
        estimated_output_tokens: 10,
        ..estimate_for("planner")
    };
    assert!(
        gate.check_budgets_against(&estimate, &gate.live_budgets(), &tracker)
            .is_none(),
        "a model budget refused a call that names no model"
    );
}
