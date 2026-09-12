use super::*;
use crate::openhuman::config::Config;
use std::sync::{Arc, Barrier};
use tempfile::TempDir;

fn test_config(dir: &TempDir) -> Config {
    let mut config = Config::default();
    config.workspace_dir = dir.path().to_path_buf();
    config
}

fn sample_notification(id: &str, provider: &str) -> IntegrationNotification {
    IntegrationNotification {
        id: id.to_string(),
        provider: provider.to_string(),
        account_id: None,
        title: "Test notification".to_string(),
        body: "Test body".to_string(),
        raw_payload: serde_json::json!({"test": true}),
        importance_score: None,
        triage_action: None,
        triage_reason: None,
        status: NotificationStatus::Unread,
        received_at: Utc::now(),
        scored_at: None,
    }
}

#[test]
fn insert_and_list_roundtrip() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    let n = sample_notification("n1", "gmail");
    insert(&config, &n).unwrap();

    let items = list(&config, 10, 0, None, None).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "n1");
    assert_eq!(items[0].provider, "gmail");
}

#[test]
fn unread_count_increments_on_insert_and_decrements_on_read() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    assert_eq!(unread_count(&config).unwrap(), 0);
    insert(&config, &sample_notification("a", "slack")).unwrap();
    insert(&config, &sample_notification("b", "slack")).unwrap();
    assert_eq!(unread_count(&config).unwrap(), 2);

    mark_read(&config, "a").unwrap();
    assert_eq!(unread_count(&config).unwrap(), 1);
}

#[test]
fn update_triage_fills_scoring_fields() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    insert(&config, &sample_notification("t1", "gmail")).unwrap();
    update_triage(&config, "t1", 0.9, "escalate", "important email").unwrap();

    let items = list(&config, 10, 0, None, None).unwrap();
    assert_eq!(items[0].importance_score, Some(0.9));
    assert_eq!(items[0].triage_action.as_deref(), Some("escalate"));
    assert_eq!(items[0].triage_reason.as_deref(), Some("important email"));
    assert!(items[0].scored_at.is_some());
}

#[test]
fn provider_filter_works() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    insert(&config, &sample_notification("g1", "gmail")).unwrap();
    insert(&config, &sample_notification("s1", "slack")).unwrap();

    let gmail = list(&config, 10, 0, Some("gmail"), None).unwrap();
    assert_eq!(gmail.len(), 1);
    assert_eq!(gmail[0].provider, "gmail");
}

#[test]
fn insert_if_not_recent_skips_duplicate() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    let n = sample_notification("dup-a", "slack");
    assert!(insert_if_not_recent(&config, &n).unwrap());

    let n2 = sample_notification("dup-b", "slack");
    assert!(!insert_if_not_recent(&config, &n2).unwrap());
}

#[test]
fn insert_if_not_recent_rejects_expired_window_only() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let mut old = sample_notification("old1", "slack");
    old.received_at = Utc::now() - chrono::Duration::seconds(120);
    insert(&config, &old).unwrap();

    let fresh_same_content = sample_notification("fresh1", "slack");
    assert!(insert_if_not_recent(&config, &fresh_same_content).unwrap());
}

#[test]
fn insert_if_not_recent_is_atomic_under_concurrent_calls() {
    let dir = TempDir::new().unwrap();
    let config = Arc::new(test_config(&dir));
    let gate = Arc::new(Barrier::new(3));

    let run = |id: &'static str, gate: Arc<Barrier>, config: Arc<Config>| {
        std::thread::spawn(move || {
            let n = sample_notification(id, "slack");
            gate.wait();
            insert_if_not_recent(&config, &n)
        })
    };

    let t1 = run("race-a", Arc::clone(&gate), Arc::clone(&config));
    let t2 = run("race-b", Arc::clone(&gate), Arc::clone(&config));

    gate.wait();
    let inserted_1 = t1.join().unwrap().unwrap();
    let inserted_2 = t2.join().unwrap().unwrap();

    let inserted_total = usize::from(inserted_1) + usize::from(inserted_2);
    assert_eq!(inserted_total, 1);

    let items = list(&config, 10, 0, Some("slack"), None).unwrap();
    assert_eq!(items.len(), 1);
}

#[test]
fn exists_recent_rejects_expired_notification() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    let mut n = sample_notification("old1", "slack");
    n.received_at = Utc::now() - chrono::Duration::seconds(120);
    insert(&config, &n).unwrap();

    assert!(!exists_recent(&config, "slack", None, "Test notification", "Test body").unwrap());
}

#[test]
fn settings_roundtrip_defaults_and_upsert() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let defaults = get_settings(&config, "gmail").unwrap();
    assert_eq!(defaults.provider, "gmail");
    assert!(defaults.enabled);
    assert_eq!(defaults.importance_threshold, 0.0);
    assert!(defaults.route_to_orchestrator);

    upsert_settings(
        &config,
        &NotificationSettings {
            provider: "gmail".to_string(),
            enabled: false,
            importance_threshold: 0.75,
            route_to_orchestrator: false,
        },
    )
    .unwrap();

    let updated = get_settings(&config, "gmail").unwrap();
    assert!(!updated.enabled);
    assert_eq!(updated.importance_threshold, 0.75);
    assert!(!updated.route_to_orchestrator);
}

#[test]
fn exists_recent_detects_with_and_without_account_id() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let mut n = sample_notification("acct-1", "slack");
    n.account_id = Some("acct-main".to_string());
    insert(&config, &n).unwrap();

    assert!(exists_recent(
        &config,
        "slack",
        Some("acct-main"),
        "Test notification",
        "Test body"
    )
    .unwrap());
    assert!(!exists_recent(
        &config,
        "slack",
        Some("acct-other"),
        "Test notification",
        "Test body"
    )
    .unwrap());

    let n_null = sample_notification("acct-null", "slack");
    insert(&config, &n_null).unwrap();
    assert!(exists_recent(&config, "slack", None, "Test notification", "Test body").unwrap());
}

#[test]
fn mark_dismissed_and_mark_acted_report_match_and_update_status() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    insert(&config, &sample_notification("m1", "gmail")).unwrap();
    insert(&config, &sample_notification("m2", "gmail")).unwrap();

    assert!(mark_dismissed(&config, "m1").unwrap());
    assert!(mark_acted(&config, "m2").unwrap());
    assert!(!mark_dismissed(&config, "missing").unwrap());
    assert!(!mark_acted(&config, "missing").unwrap());

    let items = list(&config, 10, 0, Some("gmail"), None).unwrap();
    let m1 = items.iter().find(|n| n.id == "m1").unwrap();
    let m2 = items.iter().find(|n| n.id == "m2").unwrap();
    assert_eq!(m1.status, NotificationStatus::Dismissed);
    assert_eq!(m2.status, NotificationStatus::Acted);
}

#[test]
fn stats_returns_correct_aggregates() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    insert(&config, &sample_notification("s1", "gmail")).unwrap();
    insert(&config, &sample_notification("s2", "gmail")).unwrap();
    insert(&config, &sample_notification("s3", "slack")).unwrap();
    update_triage(&config, "s2", 0.9, "escalate", "urgent").unwrap();
    update_triage(&config, "s3", 0.2, "drop", "noise").unwrap();
    mark_read(&config, "s2").unwrap();

    let out = stats(&config).unwrap();
    assert_eq!(out.total, 3);
    assert_eq!(out.unread, 2);
    assert_eq!(out.unscored, 1);
    assert_eq!(out.by_provider.get("gmail"), Some(&2));
    assert_eq!(out.by_provider.get("slack"), Some(&1));
    assert_eq!(out.by_action.get("escalate"), Some(&1));
    assert_eq!(out.by_action.get("drop"), Some(&1));
}
