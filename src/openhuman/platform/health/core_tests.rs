use super::*;

fn unique_component(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4())
}

#[test]
fn mark_component_ok_initializes_component_state() {
    let component = unique_component("health-ok");

    mark_component_ok(&component);

    let snapshot = snapshot();
    let entry = snapshot
        .components
        .get(&component)
        .expect("component should be present after mark_component_ok");

    assert_eq!(entry.status, "ok");
    assert!(entry.last_ok.is_some());
    assert!(entry.last_error.is_none());
}

#[test]
fn mark_component_error_then_ok_clears_last_error() {
    let component = unique_component("health-error");

    mark_component_error(&component, "first failure");
    let error_snapshot = snapshot();
    let errored = error_snapshot
        .components
        .get(&component)
        .expect("component should exist after mark_component_error");
    assert_eq!(errored.status, "error");
    assert_eq!(errored.last_error.as_deref(), Some("first failure"));

    mark_component_ok(&component);
    let recovered_snapshot = snapshot();
    let recovered = recovered_snapshot
        .components
        .get(&component)
        .expect("component should exist after recovery");
    assert_eq!(recovered.status, "ok");
    assert!(recovered.last_error.is_none());
    assert!(recovered.last_ok.is_some());
}

#[test]
fn bump_component_restart_increments_counter() {
    let component = unique_component("health-restart");

    bump_component_restart(&component);
    bump_component_restart(&component);

    let snapshot = snapshot();
    let entry = snapshot
        .components
        .get(&component)
        .expect("component should exist after restart bump");

    assert_eq!(entry.restart_count, 2);
}

#[test]
fn snapshot_json_contains_registered_component_fields() {
    let component = unique_component("health-json");

    mark_component_ok(&component);

    let json = snapshot_json();
    let component_json = &json["components"][&component];

    assert_eq!(component_json["status"], "ok");
    assert!(component_json["updated_at"].as_str().is_some());
    assert!(component_json["last_ok"].as_str().is_some());
    assert!(json["uptime_seconds"].as_u64().is_some());
}

// ── Critical-component verdict (#3312) ────────────────────────────────

fn component(status: &str) -> ComponentHealth {
    ComponentHealth {
        status: status.to_string(),
        updated_at: "2026-06-10T00:00:00Z".to_string(),
        last_ok: None,
        last_error: None,
        restart_count: 0,
    }
}

/// Build a synthetic snapshot from `(name, status)` pairs — lets the
/// verdict be tested without mutating the process-global registry.
fn snapshot_of(components: &[(&str, &str)]) -> HealthSnapshot {
    HealthSnapshot {
        pid: 1,
        updated_at: "2026-06-10T00:00:00Z".to_string(),
        uptime_seconds: 0,
        components: components
            .iter()
            .map(|(n, s)| ((*n).to_string(), component(s)))
            .collect(),
    }
}

#[test]
fn critical_set_membership() {
    assert!(is_critical_component("core"));
    assert!(is_critical_component("memory_tree_db"));
    assert!(!is_critical_component("scheduler"));
    assert!(!is_critical_component("channels"));
    assert!(!is_critical_component("update_checker"));
}

#[test]
fn all_ok_is_healthy_and_not_degraded() {
    let v = verdict(&snapshot_of(&[("core", "ok"), ("scheduler", "ok")]));
    assert!(v.healthy);
    assert!(!v.degraded);
    assert!(v.critical_unhealthy.is_empty());
    assert!(v.degraded_components.is_empty());
}

#[test]
fn noncritical_failure_stays_healthy_but_degraded() {
    // The exact #3312 case: scheduler in error must NOT 503 the container.
    let v = verdict(&snapshot_of(&[("core", "ok"), ("scheduler", "error")]));
    assert!(v.healthy, "a degraded background service must not 503");
    assert!(v.degraded);
    assert_eq!(v.degraded_components, vec!["scheduler".to_string()]);
    assert!(v.critical_unhealthy.is_empty());
}

#[test]
fn critical_failure_is_unhealthy() {
    let v = verdict(&snapshot_of(&[("memory_tree_db", "error")]));
    assert!(
        !v.healthy,
        "a critical component failure 503s the container"
    );
    assert_eq!(v.critical_unhealthy, vec!["memory_tree_db".to_string()]);
}

#[test]
fn mixed_failures_report_both_buckets_and_503() {
    let v = verdict(&snapshot_of(&[
        ("core", "error"),
        ("scheduler", "error"),
        ("channels", "ok"),
    ]));
    assert!(!v.healthy);
    assert_eq!(v.critical_unhealthy, vec!["core".to_string()]);
    assert_eq!(v.degraded_components, vec!["scheduler".to_string()]);
}

#[test]
fn starting_status_is_treated_as_healthy() {
    // Boot grace: a not-yet-reported component must not 503 nor degrade.
    let v = verdict(&snapshot_of(&[
        ("core", "starting"),
        ("scheduler", "starting"),
    ]));
    assert!(v.healthy);
    assert!(!v.degraded);
}
