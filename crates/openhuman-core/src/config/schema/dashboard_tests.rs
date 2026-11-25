use super::*;

#[test]
fn construction_and_empty_json_preserve_event_stream_defaults() {
    for cfg in [
        DashboardConfig::default(),
        serde_json::from_value(serde_json::json!({})).unwrap(),
    ] {
        assert!(cfg.event_stream.enabled);
        assert_eq!(cfg.event_stream.max_entries, 200);
        assert_eq!(cfg.event_stream.new_entries, "top");
    }
}

#[test]
fn deserialize_custom_values() {
    let cfg: DashboardConfig = serde_json::from_value(serde_json::json!({
        "event_stream": {
            "enabled": false,
            "max_entries": 500,
            "new_entries": "bottom"
        }
    }))
    .unwrap();
    assert!(!cfg.event_stream.enabled);
    assert_eq!(cfg.event_stream.max_entries, 500);
    assert_eq!(cfg.event_stream.new_entries, "bottom");
}

#[test]
fn model_health_defaults_match_spec() {
    let mh = ModelHealthConfig::default();
    assert!(mh.enabled);
    assert!((mh.hallucination_threshold - 0.10).abs() < f64::EPSILON);
    assert_eq!(mh.min_tasks_for_rating, 10);
    assert_eq!(mh.evaluation_window_tasks, 50);
}
