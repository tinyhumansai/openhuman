use super::*;

#[test]
fn defaults_match_issue_spec() {
    let cfg = DashboardConfig::default();
    assert!(cfg.event_stream.enabled);
    assert_eq!(cfg.event_stream.max_entries, 200);
    assert_eq!(cfg.event_stream.new_entries, "top");
}

#[test]
fn deserialize_from_empty_json() {
    let cfg: DashboardConfig = serde_json::from_value(serde_json::json!({})).unwrap();
    assert!(cfg.event_stream.enabled);
    assert_eq!(cfg.event_stream.max_entries, 200);
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
fn dashboard_config_defaults_enable_local_diagram_viewer() {
    let config = DashboardConfig::default();

    assert!(config.diagram_viewer.enabled);
    assert_eq!(
        config.diagram_viewer.source_url,
        "http://localhost:8787/workspace/diagrams/latest.png"
    );
    assert_eq!(config.diagram_viewer.refresh_interval_seconds, 10);
}

#[test]
fn diagram_viewer_partial_toml_uses_missing_defaults() {
    let config: DashboardConfig =
        toml::from_str("[diagram_viewer]\nsource_url = \"http://localhost:9000/latest.svg\"")
            .expect("dashboard config should deserialize");

    assert!(config.diagram_viewer.enabled);
    assert_eq!(
        config.diagram_viewer.source_url,
        "http://localhost:9000/latest.svg"
    );
    assert_eq!(config.diagram_viewer.refresh_interval_seconds, 10);
}

#[test]
fn model_health_defaults_match_spec() {
    let mh = ModelHealthConfig::default();
    assert!(mh.enabled);
    assert!((mh.hallucination_threshold - 0.10).abs() < f64::EPSILON);
    assert_eq!(mh.min_tasks_for_rating, 10);
    assert_eq!(mh.evaluation_window_tasks, 50);
}
