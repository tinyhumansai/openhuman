use super::*;
use crate::platform::cost::types::TokenUsage;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 15, 12, 0, 0).unwrap()
}

fn spend(day: u32, model: &str, agent: Option<&str>, thread: Option<&str>, usd: f64) -> CostRecord {
    let mut usage = TokenUsage::new(model, 1000, 500, 0.0, 0.0);
    usage.cost_usd = usd;
    usage.timestamp = Utc.with_ymd_and_hms(2026, 10, day, 9, 0, 0).unwrap();
    usage.scope = UsageScope {
        agent_id: agent.map(str::to_owned),
        thread_id: thread.map(str::to_owned),
        ..UsageScope::default()
    };
    CostRecord::new("s", usage)
}

fn policy(
    scope: BudgetScope,
    period: BudgetPeriod,
    max_usd: f64,
    action: BudgetAction,
) -> BudgetPolicy {
    BudgetPolicy {
        name: Some("test".into()),
        scope,
        matches: None,
        period,
        max_usd: Some(max_usd),
        max_tokens: None,
        warn_fraction: 0.8,
        action,
    }
}

fn call<'a>(model: &'a str, scope: &'a UsageScope) -> CallUnderCheck<'a> {
    CallUnderCheck {
        model,
        scope,
        estimated_usd: 0.0,
        estimated_tokens: 0,
    }
}

#[test]
fn period_starts_are_midnight_and_the_first_of_the_month() {
    assert_eq!(
        period_start(BudgetPeriod::Day, now()),
        Utc.with_ymd_and_hms(2026, 10, 15, 0, 0, 0).unwrap()
    );
    assert_eq!(
        period_start(BudgetPeriod::Month, now()),
        Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap()
    );
}

#[test]
fn a_global_monthly_budget_refuses_once_reached() {
    let records = vec![
        spend(2, "m", None, None, 6.0),
        spend(14, "m", None, None, 4.0),
    ];
    let policies = vec![policy(
        BudgetScope::Global,
        BudgetPeriod::Month,
        10.0,
        BudgetAction::Refuse,
    )];
    let scope = UsageScope::default();
    let verdict = evaluate(&policies, &records, call("m", &scope), now());
    let hit = verdict.refusal().expect("refused");
    assert!(hit.exceeded);
    assert_eq!(hit.bucket, "*");
    assert!(
        hit.refusal().starts_with("BUDGET_EXCEEDED:"),
        "{}",
        hit.refusal()
    );
}

#[test]
fn a_daily_budget_counts_only_today() {
    let records = vec![
        spend(14, "m", None, None, 9.0),
        spend(15, "m", None, None, 1.0),
    ];
    let policies = vec![policy(
        BudgetScope::Global,
        BudgetPeriod::Day,
        5.0,
        BudgetAction::Refuse,
    )];
    let scope = UsageScope::default();
    let verdict = evaluate(&policies, &records, call("m", &scope), now());
    assert!(verdict.hits.is_empty(), "{verdict:?}");
}

#[test]
fn warn_policies_and_the_warn_fraction_never_refuse() {
    let records = vec![spend(10, "m", None, None, 8.5)];
    let scope = UsageScope::default();
    let near = evaluate(
        &[policy(
            BudgetScope::Global,
            BudgetPeriod::Month,
            10.0,
            BudgetAction::Refuse,
        )],
        &records,
        call("m", &scope),
        now(),
    );
    assert_eq!(near.hits.len(), 1);
    assert!(!near.hits[0].exceeded);
    assert!(
        near.refusal().is_none(),
        "past the warn fraction is only a warning"
    );

    let over_warn_only = evaluate(
        &[policy(
            BudgetScope::Global,
            BudgetPeriod::Month,
            5.0,
            BudgetAction::Warn,
        )],
        &records,
        call("m", &scope),
        now(),
    );
    assert!(over_warn_only.hits[0].exceeded);
    assert!(over_warn_only.refusal().is_none());
}

#[test]
fn per_agent_budgets_count_each_agent_separately() {
    let records = vec![
        spend(10, "m", Some("planner"), None, 9.0),
        spend(10, "m", Some("orchestrator"), None, 1.0),
    ];
    let policies = vec![policy(
        BudgetScope::Agent,
        BudgetPeriod::Month,
        5.0,
        BudgetAction::Refuse,
    )];
    let planner = UsageScope {
        agent_id: Some("planner".into()),
        ..UsageScope::default()
    };
    let orchestrator = UsageScope {
        agent_id: Some("orchestrator".into()),
        ..UsageScope::default()
    };
    assert!(evaluate(&policies, &records, call("m", &planner), now())
        .refusal()
        .is_some());
    assert!(
        evaluate(&policies, &records, call("m", &orchestrator), now())
            .hits
            .is_empty()
    );
    // A call with no agent is outside a per-agent budget.
    let unattributed = UsageScope::default();
    assert!(
        evaluate(&policies, &records, call("m", &unattributed), now())
            .hits
            .is_empty()
    );
}

#[test]
fn a_match_limits_the_budget_to_one_bucket() {
    let records = vec![
        spend(10, "expensive", None, None, 9.0),
        spend(10, "cheap", None, None, 9.0),
    ];
    let mut only_expensive = policy(
        BudgetScope::Model,
        BudgetPeriod::Month,
        5.0,
        BudgetAction::Refuse,
    );
    only_expensive.matches = Some("expensive".into());
    let scope = UsageScope::default();
    assert!(evaluate(
        &[only_expensive.clone()],
        &records,
        call("expensive", &scope),
        now()
    )
    .refusal()
    .is_some());
    assert!(
        evaluate(&[only_expensive], &records, call("cheap", &scope), now())
            .hits
            .is_empty()
    );
}

#[test]
fn token_limits_count_input_and_output() {
    let records = vec![
        spend(10, "m", None, None, 0.0),
        spend(11, "m", None, None, 0.0),
    ];
    let mut tokens = policy(
        BudgetScope::Global,
        BudgetPeriod::Month,
        0.0,
        BudgetAction::Refuse,
    );
    tokens.max_usd = None;
    tokens.max_tokens = Some(3000);
    let scope = UsageScope::default();
    let verdict = evaluate(&[tokens], &records, call("m", &scope), now());
    let hit = verdict.refusal().expect("3000 tokens used");
    assert_eq!(hit.tokens, 3000);
    assert!(
        hit.refusal().contains("3000 of 3000 tokens"),
        "{}",
        hit.refusal()
    );
}

#[test]
fn a_policy_without_limits_is_ignored() {
    let mut empty = policy(
        BudgetScope::Global,
        BudgetPeriod::Month,
        0.0,
        BudgetAction::Refuse,
    );
    empty.max_usd = None;
    let scope = UsageScope::default();
    let verdict = evaluate(
        &[empty],
        &[spend(10, "m", None, None, 99.0)],
        call("m", &scope),
        now(),
    );
    assert!(verdict.hits.is_empty());
}

#[test]
fn earliest_start_covers_the_longest_period() {
    let policies = vec![
        policy(
            BudgetScope::Global,
            BudgetPeriod::Day,
            1.0,
            BudgetAction::Warn,
        ),
        policy(
            BudgetScope::Global,
            BudgetPeriod::Month,
            1.0,
            BudgetAction::Warn,
        ),
    ];
    assert_eq!(
        earliest_start(&policies, now()),
        Some(period_start(BudgetPeriod::Month, now()))
    );
    assert_eq!(earliest_start(&[], now()), None);
}

#[test]
fn the_config_parses_from_toml() {
    let config: crate::config::CostConfig = toml::from_str(
        r#"
[[budgets]]
name = "planner cap"
scope = "agent"
match = "planner"
period = "day"
max_usd = 2.5
action = "refuse"
"#,
    )
    .unwrap();
    let b = &config.budgets[0];
    assert_eq!(b.scope, BudgetScope::Agent);
    assert_eq!(b.matches.as_deref(), Some("planner"));
    assert_eq!(b.period, BudgetPeriod::Day);
    assert_eq!(b.action, BudgetAction::Refuse);
    assert!((b.warn_fraction - 0.8).abs() < f64::EPSILON);
    assert!(toml::from_str::<crate::config::CostConfig>("[[budgets]]\nmax_usdd = 1\n").is_err());
}

#[test]
fn token_limits_compare_exactly_and_totals_saturate() {
    let scope = UsageScope::default();
    let mut huge = spend(10, "m", None, None, 0.0);
    huge.usage.input_tokens = u64::MAX;
    huge.usage.output_tokens = u64::MAX;
    let mut limit = policy(
        BudgetScope::Global,
        BudgetPeriod::Month,
        1e9,
        BudgetAction::Refuse,
    );
    limit.max_usd = None;
    limit.max_tokens = Some(u64::MAX);
    // Two saturating records reach the limit instead of wrapping under it.
    let verdict = evaluate(
        &[limit.clone()],
        &[huge.clone(), huge],
        call("m", &scope),
        now(),
    );
    assert!(verdict.refusal().is_some(), "{verdict:?}");

    // 2^53 - 1 tokens against a 2^53 limit: equal as f64, not as integers.
    let mut near = spend(10, "m", None, None, 0.0);
    near.usage.input_tokens = (1u64 << 53) - 1;
    near.usage.output_tokens = 0;
    limit.max_tokens = Some(1u64 << 53);
    let verdict = evaluate(&[limit], &[near], call("m", &scope), now());
    assert!(verdict.refusal().is_none(), "{verdict:?}");
}

#[test]
fn an_invalid_usd_cap_fails_closed() {
    let scope = UsageScope::default();
    for bad in [-1.0, f64::NAN] {
        let mut cap = policy(
            BudgetScope::Global,
            BudgetPeriod::Month,
            0.0,
            BudgetAction::Refuse,
        );
        cap.max_usd = Some(bad);
        let verdict = evaluate(&[cap], &[], call("m", &scope), now());
        assert!(verdict.refusal().is_some(), "{bad}: {verdict:?}");
    }
}

#[test]
fn a_nan_cost_record_does_not_poison_the_total() {
    let scope = UsageScope::default();
    let mut corrupt = spend(10, "m", None, None, 0.0);
    corrupt.usage.cost_usd = f64::NAN;
    let real = spend(11, "m", None, None, 5.0);
    let cap = policy(
        BudgetScope::Global,
        BudgetPeriod::Month,
        4.0,
        BudgetAction::Refuse,
    );
    let verdict = evaluate(&[cap], &[corrupt, real], call("m", &scope), now());
    assert!(verdict.refusal().is_some(), "{verdict:?}");
}

#[test]
fn the_calls_own_estimate_counts() {
    let scope = UsageScope::default();
    let earlier = spend(10, "m", None, None, 0.9);
    let cap = policy(
        BudgetScope::Global,
        BudgetPeriod::Month,
        1.0,
        BudgetAction::Refuse,
    );
    let mut big = call("m", &scope);
    big.estimated_usd = 0.2;
    assert!(evaluate(
        std::slice::from_ref(&cap),
        std::slice::from_ref(&earlier),
        big,
        now()
    )
    .refusal()
    .is_some());
    assert!(evaluate(&[cap], &[earlier], call("m", &scope), now())
        .refusal()
        .is_none());
}

#[test]
fn a_nan_warn_fraction_falls_back_to_the_default() {
    let scope = UsageScope::default();
    let mut cap = policy(
        BudgetScope::Global,
        BudgetPeriod::Month,
        1.0,
        BudgetAction::Warn,
    );
    cap.warn_fraction = f64::NAN;
    let verdict = evaluate(
        &[cap],
        &[spend(10, "m", None, None, 0.85)],
        call("m", &scope),
        now(),
    );
    assert_eq!(verdict.hits.len(), 1, "{verdict:?}");
}
