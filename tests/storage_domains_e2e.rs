//! Paired devices, notifications and task sources on a configured storage
//! backend, end to end through their public store functions.
//!
//! Its own test binary because it installs a backend into the process-wide
//! storage slot, which would reroute every other suite's stores in a shared
//! process. The tests take turns on the slot (see `support/storage_case.rs`).

use chrono::Utc;
use openhuman_core::config::Config;
use openhuman_core::desktop::notifications::store as notifications;
use openhuman_core::desktop::notifications::types::{IntegrationNotification, NotificationStatus};
use openhuman_core::integrations::composio::contract::NormalizedTask;
use openhuman_core::integrations::task_sources::store as task_sources;
use openhuman_core::integrations::task_sources::types::{FilterSpec, ProviderSlug, SourceTarget};
use openhuman_core::security::devices::store as devices;

#[path = "support/storage_case.rs"]
mod storage_case;

use storage_case::Case;

fn notification(id: &str) -> IntegrationNotification {
    IntegrationNotification {
        id: id.to_string(),
        provider: "slack".to_string(),
        account_id: None,
        title: "New message".to_string(),
        body: "hello".to_string(),
        raw_payload: serde_json::json!({}),
        importance_score: None,
        triage_action: None,
        triage_reason: None,
        status: NotificationStatus::Unread,
        received_at: Utc::now(),
        scored_at: None,
    }
}

#[test]
fn a_configured_backend_holds_devices_notifications_and_task_sources() {
    let case = Case::open();
    let workspace = tempfile::tempdir().unwrap();
    let config = Config {
        workspace_dir: workspace.path().to_path_buf(),
        ..Config::default()
    };
    case.install();

    devices::insert_device(&config, "ch-1", "iPhone", "pk", "hash").unwrap();
    assert_eq!(devices::list_devices(&config).unwrap().len(), 1);
    assert!(devices::revoke_device(&config, "ch-1").unwrap());

    assert!(notifications::insert_if_not_recent(&config, &notification("n1")).unwrap());
    assert!(!notifications::insert_if_not_recent(&config, &notification("n2")).unwrap());
    assert_eq!(notifications::unread_count(&config).unwrap(), 1);
    notifications::mark_read(&config, "n1").unwrap();
    assert_eq!(notifications::stats(&config).unwrap().total, 1);

    let filter = FilterSpec::Github {
        repo: None,
        labels: Vec::new(),
        assignee_is_me: true,
        state: None,
        fetch_mode: Default::default(),
        extra: serde_json::json!({}),
    };
    let source = task_sources::add_source(
        &config,
        ProviderSlug::Github,
        None,
        None,
        filter,
        600,
        SourceTarget::TodoOnly,
        10,
    )
    .unwrap();
    let task = NormalizedTask {
        external_id: "1".into(),
        title: "Fix it".into(),
        ..Default::default()
    };
    task_sources::mark_ingested(&config, &source.id, &task).unwrap();
    assert!(task_sources::was_ingested(&config, &source.id, "1").unwrap());
    assert_eq!(task_sources::list_sources(&config).unwrap().len(), 1);

    for db in [
        "devices/devices.db",
        "notifications/notifications.db",
        "task_sources/sources.db",
    ] {
        assert!(!workspace.path().join(db).exists(), "{db} was not written");
    }

    // Without a backend the workspace's own default database is used.
    assert!(openhuman_core::storage::clear());
    assert!(devices::list_devices(&config).unwrap().is_empty());
    assert_eq!(notifications::unread_count(&config).unwrap(), 0);
    assert!(task_sources::list_sources(&config).unwrap().is_empty());
    assert!(workspace.path().join("storage/storage.db").exists());
}
