use super::*;
use crate::platform::cost::types::{TokenUsage, UsageScope};
use chrono::TimeZone;

fn record(
    at: (u32, u32),
    model: &str,
    thread: Option<&str>,
    agent: Option<&str>,
    input: u64,
    cached: u64,
    cost: f64,
    charged: bool,
) -> CostRecord {
    let mut usage = TokenUsage::new(model, input, 10, 0.0, 0.0);
    usage.cached_input_tokens = cached;
    usage.cost_usd = cost;
    usage.cost_source = if charged {
        CostSource::ProviderCharged
    } else {
        CostSource::Estimated
    };
    usage.timestamp = Utc.with_ymd_and_hms(2026, 10, at.0, at.1, 0, 0).unwrap();
    usage.scope = UsageScope {
        thread_id: thread.map(str::to_owned),
        agent_id: agent.map(str::to_owned),
        provider: Some("openhuman".into()),
        origin: Some("web_chat".into()),
        ..UsageScope::default()
    };
    CostRecord::new("s", usage)
}

fn window() -> (DateTime<Utc>, DateTime<Utc>) {
    (
        Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2026, 10, 31, 0, 0, 0).unwrap(),
    )
}

fn sample() -> Vec<CostRecord> {
    vec![
        record(
            (1, 9),
            "anthropic/claude-sonnet-4-5",
            Some("t1"),
            Some("orchestrator"),
            1000,
            0,
            0.50,
            true,
        ),
        record(
            (1, 10),
            "anthropic/claude-sonnet-4-5",
            Some("t1"),
            Some("orchestrator"),
            1000,
            800,
            0.20,
            true,
        ),
        record(
            (2, 9),
            "openai/gpt-5-mini",
            Some("t2"),
            Some("planner"),
            500,
            100,
            0.05,
            false,
        ),
        record(
            (2, 10),
            "openai/gpt-5-mini",
            None,
            None,
            100,
            0,
            0.01,
            false,
        ),
    ]
}

#[test]
fn totals_cover_every_admitted_call() {
    let (from, to) = window();
    let report = build_report(&sample(), from, to, &[], &ReportFilter::default());
    let t = &report.totals;
    assert_eq!(t.calls, 4);
    assert_eq!(t.input_tokens, 2600);
    assert_eq!(t.cached_input_tokens, 900);
    assert!((t.cost_usd - 0.76).abs() < 1e-9);
    assert!((t.charged_usd - 0.70).abs() < 1e-9);
    assert!((t.estimated_usd - 0.06).abs() < 1e-9);
    assert!((t.cache_hit_ratio - 900.0 / 2600.0).abs() < 1e-9);
    assert!(report.rows.is_empty(), "no grouping, no rows");
}

#[test]
fn groups_by_agent_most_expensive_first() {
    let (from, to) = window();
    let report = build_report(
        &sample(),
        from,
        to,
        &[GroupKey::Agent],
        &ReportFilter::default(),
    );
    let agents: Vec<_> = report.rows.iter().map(|r| r.key["agent"].clone()).collect();
    assert_eq!(agents, vec!["orchestrator", "planner", UNKNOWN]);
    let orchestrator = &report.rows[0];
    assert_eq!(orchestrator.calls, 2);
    assert!((orchestrator.cache_hit_ratio - 0.4).abs() < 1e-9);
}

#[test]
fn groups_by_several_keys_and_dedupes_them() {
    let (from, to) = window();
    let report = build_report(
        &sample(),
        from,
        to,
        &[GroupKey::Day, GroupKey::Route, GroupKey::Day],
        &ReportFilter::default(),
    );
    assert_eq!(report.group_by, vec![GroupKey::Day, GroupKey::Route]);
    let keys: Vec<_> = report
        .rows
        .iter()
        .map(|r| format!("{}|{}", r.key["day"], r.key["route"]))
        .collect();
    assert!(keys.contains(&"2026-10-01|byok".to_string()), "{keys:?}");
    assert_eq!(report.rows.len(), 2, "{keys:?}");
}

#[test]
fn week_and_month_keys() {
    let r = &sample()[0];
    assert_eq!(GroupKey::Week.value(r), "2026-W40");
    assert_eq!(GroupKey::Month.value(r), "2026-10");
}

#[test]
fn filters_match_every_set_field() {
    let (from, to) = window();
    let only_t1 = ReportFilter {
        thread_id: Some("t1".into()),
        ..ReportFilter::default()
    };
    assert_eq!(
        build_report(&sample(), from, to, &[], &only_t1)
            .totals
            .calls,
        2
    );
    let none = ReportFilter {
        thread_id: Some("t1".into()),
        agent_id: Some("planner".into()),
        ..ReportFilter::default()
    };
    assert_eq!(
        build_report(&sample(), from, to, &[], &none).totals.calls,
        0
    );
    let model = ReportFilter {
        model: Some("openai/gpt-5-mini".into()),
        ..ReportFilter::default()
    };
    assert_eq!(
        build_report(&sample(), from, to, &[], &model).totals.calls,
        2
    );
}

#[test]
fn records_from_before_attribution_group_as_unknown() {
    let mut old = sample().remove(0);
    old.usage.scope = UsageScope::default();
    let (from, to) = window();
    let report = build_report(
        &[old],
        from,
        to,
        &[GroupKey::Thread],
        &ReportFilter::default(),
    );
    assert_eq!(report.rows[0].key["thread"], UNKNOWN);
}

#[test]
fn a_repeat_call_without_a_cache_hit_is_cold() {
    let (from, to) = window();
    let mut records = sample();
    records.push(record(
        (3, 9),
        "anthropic/claude-sonnet-4-5",
        Some("t1"),
        Some("orchestrator"),
        1000,
        0,
        0.5,
        true,
    ));
    let report = build_cache_report(&records, from, to, &ReportFilter::default());
    let cold: Vec<_> = report.calls.iter().filter(|c| c.cold).collect();
    assert_eq!(report.cold_calls, 1, "{:?}", report.calls);
    assert_eq!(cold[0].timestamp.day(), 3);
    assert!(
        !report.calls[0].cold,
        "the first call of a thread cannot hit"
    );
    assert!(report.calls[1].cache_hit_ratio > 0.79);
}

#[test]
fn cache_report_prices_the_uncached_premium_for_known_models() {
    let (from, to) = window();
    let records = vec![record(
        (1, 9),
        "anthropic/claude-sonnet-4-5",
        Some("t1"),
        None,
        1_000_000,
        0,
        3.0,
        true,
    )];
    let report = build_cache_report(&records, from, to, &ReportFilter::default());
    let price = tinyagents_registry::pricing::lookup("anthropic/claude-sonnet-4-5");
    match price {
        Some(p) => assert!(
            (report.uncached_premium_usd - (p.input_per_mtok_usd - p.cached_input_per_mtok_usd))
                .abs()
                < 1e-9
        ),
        None => assert_eq!(report.uncached_premium_usd, 0.0),
    }
}

#[test]
fn a_model_switch_in_a_thread_is_not_a_cold_call() {
    let (from, to) = window();
    let records = vec![
        record((2, 9), "model/a", Some("t9"), Some("x"), 1000, 0, 0.1, true),
        // First call on another model: its cache could not have been warm.
        record(
            (2, 10),
            "model/b",
            Some("t9"),
            Some("x"),
            1000,
            0,
            0.1,
            true,
        ),
        // Same model again, still nothing cached: genuinely cold.
        record(
            (2, 11),
            "model/b",
            Some("t9"),
            Some("x"),
            1000,
            0,
            0.1,
            true,
        ),
    ];
    let report = build_cache_report(&records, from, to, &ReportFilter::default());
    assert_eq!(report.cold_calls, 1);
    assert_eq!(
        report.calls.iter().map(|c| c.cold).collect::<Vec<_>>(),
        vec![false, false, true]
    );
}

#[test]
fn an_estimated_cost_lands_in_estimated_usd_not_charged() {
    use crate::inference::provider::BilledUsage;
    let estimated = BilledUsage::from_counts(100, 10).with_estimated_usd(0.25);
    let usage = crate::platform::cost::global::build_token_usage("model/a", &estimated).unwrap();
    assert_eq!(usage.cost_source, CostSource::Estimated);
    let charged = BilledUsage::from_counts(100, 10).with_charged_usd(0.25);
    let usage = crate::platform::cost::global::build_token_usage("model/a", &charged).unwrap();
    assert_eq!(usage.cost_source, CostSource::ProviderCharged);
}

#[test]
fn embedding_batches_stay_out_of_the_cache_report() {
    let (from, to) = window();
    let mut embedding = record(
        (2, 9),
        "voyage/voyage-3",
        Some("t1"),
        None,
        5000,
        0,
        0.0,
        false,
    );
    embedding.usage.scope.origin = Some(EMBEDDING_ORIGIN.into());
    let mut records = sample();
    records.push(embedding);
    let with = build_cache_report(&records, from, to, &ReportFilter::default());
    let without = build_cache_report(&sample(), from, to, &ReportFilter::default());
    assert_eq!(with.calls.len(), without.calls.len());
    assert_eq!(with.cache_hit_ratio, without.cache_hit_ratio);
}

#[test]
fn a_provider_switch_in_a_thread_is_not_a_cold_call() {
    let (from, to) = window();
    let first = record((2, 9), "model/a", Some("t9"), Some("x"), 1000, 0, 0.1, true);
    let mut second = record(
        (2, 10),
        "model/a",
        Some("t9"),
        Some("x"),
        1000,
        0,
        0.1,
        true,
    );
    second.usage.scope.provider = Some("openrouter".into());
    let report = build_cache_report(&[first, second], from, to, &ReportFilter::default());
    assert_eq!(report.cold_calls, 0, "{:?}", report.calls);
}

#[test]
fn same_time_calls_classify_the_same_in_any_order() {
    let (from, to) = window();
    let mut records: Vec<CostRecord> = (0..3)
        .map(|i| {
            let mut r = record(
                (2, 9),
                "model/a",
                Some("t9"),
                Some("x"),
                if i == 0 { 0 } else { 1000 },
                0,
                0.1,
                true,
            );
            r.id = format!("r{i}");
            r
        })
        .collect();
    let forward = build_cache_report(&records, from, to, &ReportFilter::default());
    records.reverse();
    let backward = build_cache_report(&records, from, to, &ReportFilter::default());
    assert_eq!(forward.cold_calls, backward.cold_calls);
    assert_eq!(
        forward.calls.iter().map(|c| c.cold).collect::<Vec<_>>(),
        backward.calls.iter().map(|c| c.cold).collect::<Vec<_>>()
    );
}

#[test]
fn a_misspelt_filter_key_does_not_deserialize() {
    let parsed: Result<ReportFilter, _> = serde_json::from_value(serde_json::json!({
        "threadId": "t1"
    }));
    assert!(parsed.is_err());
}

#[test]
fn a_thread_begun_before_the_window_is_already_seen() {
    let (from, to) = window();
    // September 30th: before the window, only seeds the thread's cache.
    let mut before = record((1, 9), "model/a", Some("t9"), Some("x"), 1000, 0, 0.1, true);
    before.usage.timestamp = from - chrono::Duration::hours(2);
    let inside = record((1, 9), "model/a", Some("t9"), Some("x"), 1000, 0, 0.1, true);
    let report = build_cache_report(&[before, inside], from, to, &ReportFilter::default());
    assert_eq!(report.calls.len(), 1, "the earlier call is not reported");
    assert_eq!(report.cold_calls, 1, "its first call in range is a repeat");
}
