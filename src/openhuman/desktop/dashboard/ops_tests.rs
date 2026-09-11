use super::*;

fn cfg_with_models() -> Config {
    let mut cfg = Config::default();
    cfg.model_registry = vec![
        crate::openhuman::config::schema::ModelRegistryEntry {
            id: "deepseek-v3.2".to_string(),
            provider: "SiliconFlow".to_string(),
            cost_per_1m_output: 0.33,
            vision: false,
            ..Default::default()
        },
        crate::openhuman::config::schema::ModelRegistryEntry {
            id: "qwen-2.5-8b".to_string(),
            provider: "OpenRouter".to_string(),
            cost_per_1m_output: 0.09,
            vision: true,
            ..Default::default()
        },
    ];
    cfg
}

#[test]
fn maps_registry_entries_with_placeholder_metrics() {
    let cfg = cfg_with_models();
    let outcome = model_health(&cfg).expect("enabled");
    let resp = &outcome.value;
    assert_eq!(resp.models.len(), 2);

    let first = &resp.models[0];
    assert_eq!(first.id, "deepseek-v3.2");
    assert_eq!(first.provider, "SiliconFlow");
    assert!((first.cost_per_1m_output - 0.33).abs() < f64::EPSILON);
    assert!(!first.vision);
    // Placeholder telemetry fields — assert the contract.
    assert!(first.quality_score.is_none());
    assert!(first.hallucination_rate.is_none());
    assert_eq!(first.agents_using, 0);
    assert_eq!(first.tasks_evaluated, 0);

    let second = &resp.models[1];
    assert_eq!(second.id, "qwen-2.5-8b");
    assert!(second.vision);
}

#[test]
fn surfaces_config_thresholds() {
    let cfg = cfg_with_models();
    let outcome = model_health(&cfg).expect("enabled");
    let resp = &outcome.value;
    assert!((resp.config.hallucination_threshold - 0.10).abs() < f64::EPSILON);
    assert_eq!(resp.config.min_tasks_for_rating, 10);
    assert_eq!(resp.config.evaluation_window_tasks, 50);
}

#[test]
fn disabled_feature_errors() {
    let mut cfg = cfg_with_models();
    cfg.dashboard.model_health.enabled = false;
    let err = model_health(&cfg).expect_err("disabled");
    assert!(err.contains("disabled"), "unexpected error: {err}");
}

#[test]
fn empty_registry_returns_empty_models() {
    let mut cfg = Config::default();
    cfg.model_registry.clear();
    let outcome = model_health(&cfg).expect("enabled");
    assert!(outcome.value.models.is_empty());
}
