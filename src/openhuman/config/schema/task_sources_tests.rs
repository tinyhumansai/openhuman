use super::*;

#[test]
fn defaults_are_sane() {
    let c = TaskSourcesConfig::default();
    assert!(c.enabled);
    assert_eq!(c.default_interval_secs, 1800);
    assert_eq!(c.max_tasks_per_fetch, 25);
    assert!(c.auto_proactive);
}

#[test]
fn deserializes_from_empty_table() {
    let c: TaskSourcesConfig = serde_json::from_str("{}").unwrap();
    assert!(c.enabled);
    assert_eq!(c.default_interval_secs, 1800);
}

#[test]
fn partial_override_keeps_other_defaults() {
    let c: TaskSourcesConfig = serde_json::from_str(r#"{"enabled": false}"#).unwrap();
    assert!(!c.enabled);
    assert_eq!(c.max_tasks_per_fetch, 25);
}
