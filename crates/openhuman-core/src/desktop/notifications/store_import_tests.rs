use super::*;
use crate::desktop::notifications::store as live;
use crate::desktop::notifications::types::{
    CoreNotificationCategory, CoreNotificationEvent, IntegrationNotification, NotificationStatus,
};
use crate::storage::import::{run_with, Outcome};
use chrono::{Duration, Utc};
use tempfile::TempDir;

fn workspace() -> (TempDir, Config) {
    let dir = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    (dir, config)
}

fn import(config: &Config) -> Outcome {
    let backend = crate::storage::local::backend_for(&config.workspace_dir).unwrap();
    run_with(&backend, config, &[super::super::import_source()])
        .remove(0)
        .outcome
        .unwrap()
}

/// Writes a notification row the way the retired store did.
fn seed_note(
    config: &Config,
    n: &IntegrationNotification,
    score: Option<(f64, &str)>,
    status: &str,
) {
    with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO integration_notifications
                (id, provider, account_id, title, body, raw_payload, importance_score,
                 triage_action, triage_reason, status, received_at, scored_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                n.id,
                n.provider,
                n.account_id,
                n.title,
                n.body,
                n.raw_payload.to_string(),
                score.map(|(s, _)| s),
                score.map(|(_, a)| a),
                score.map(|_| "important"),
                status,
                n.received_at.to_rfc3339(),
                score.map(|_| Utc::now().to_rfc3339()),
            ],
        )?;
        Ok(())
    })
    .unwrap();
}

fn seed_core(config: &Config, e: &CoreNotificationEvent, read: bool) {
    with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO core_notifications (id, payload, timestamp_ms, read, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                e.id,
                serde_json::to_string(e).unwrap(),
                e.timestamp_ms as i64,
                i64::from(read),
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    })
    .unwrap();
}

fn note(id: &str, provider: &str, secs_ago: i64) -> IntegrationNotification {
    IntegrationNotification {
        id: id.to_string(),
        provider: provider.to_string(),
        account_id: Some("acct".into()),
        title: format!("title {id}"),
        body: format!("body {id}"),
        raw_payload: serde_json::json!({ "k": id }),
        importance_score: None,
        triage_action: None,
        triage_reason: None,
        status: NotificationStatus::Unread,
        received_at: Utc::now() - Duration::seconds(secs_ago),
        scored_at: None,
    }
}

fn event(id: &str, ts: u64) -> CoreNotificationEvent {
    CoreNotificationEvent {
        id: id.to_string(),
        category: CoreNotificationCategory::Agents,
        title: "Cron job completed".into(),
        body: "done".into(),
        deep_link: None,
        timestamp_ms: ts,
        actions: None,
        workspace: None,
        workspace_revision: None,
    }
}

#[test]
fn legacy_notifications_are_imported_once_and_the_file_is_left_alone() {
    let (_dir, config) = workspace();
    seed_note(&config, &note("n-old", "slack", 60), None, "unread");
    seed_note(
        &config,
        &note("n-new", "gmail", 0),
        Some((0.8, "react")),
        "unread",
    );
    seed_note(&config, &note("n-read", "gmail", 30), None, "read");
    with_connection(&config, |conn| {
        conn.execute(
            "INSERT INTO notification_settings
                (provider, enabled, importance_threshold, route_to_orchestrator)
             VALUES ('slack', 0, 0.5, 0)",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    seed_core(&config, &event("e1", 1_000), true);
    seed_core(&config, &event("e2", 2_000), false);
    let db = db_path(&config);
    let before = std::fs::read(&db).unwrap();

    assert!(matches!(
        import(&config),
        Outcome::Imported {
            found: 6,
            written: 6
        }
    ));

    let listed = live::list(&config, 10, 0, None, None).unwrap();
    let ids: Vec<_> = listed.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, ["n-new", "n-read", "n-old"]);
    let scored = &listed[0];
    assert_eq!(scored.importance_score, Some(0.8));
    assert_eq!(scored.triage_action.as_deref(), Some("react"));
    assert_eq!(scored.raw_payload["k"], "n-new");
    assert_eq!(scored.account_id.as_deref(), Some("acct"));
    assert_eq!(listed[1].status, NotificationStatus::Read);
    assert_eq!(live::unread_count(&config).unwrap(), 2);
    let settings = live::get_settings(&config, "slack").unwrap();
    assert!(!settings.enabled);
    assert!(!settings.route_to_orchestrator);
    assert!((settings.importance_threshold - 0.5).abs() < f32::EPSILON);
    let core = live::list_core_notifications(&config, false, 10).unwrap();
    let core_ids: Vec<_> = core.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(core_ids, ["e2", "e1"]);
    assert_eq!(live::unread_core_notification_count(&config).unwrap(), 1);
    assert_eq!(
        std::fs::read(&db).unwrap(),
        before,
        "notifications.db untouched"
    );

    // A second run does not import again.
    live::mark_dismissed(&config, "n-old").unwrap();
    assert!(matches!(import(&config), Outcome::AlreadyDone));
    let after = live::list(&config, 10, 0, None, None).unwrap();
    let old = after.iter().find(|n| n.id == "n-old").unwrap();
    assert_eq!(old.status, NotificationStatus::Dismissed);
    assert_eq!(after.len(), 3);
}

#[test]
fn core_notifications_survive_a_moved_workspace() {
    let (dir, config) = workspace();
    seed_core(&config, &event("e1", 1_000), false);
    import(&config);
    live::insert_core_notification(&config, &event("e2", 2_000)).unwrap();
    crate::storage::local::forget(&config.workspace_dir);

    // The workspace is moved; its storage directory comes along.
    let moved_root = TempDir::new().unwrap();
    let moved_dir = moved_root.path().join("moved");
    std::fs::rename(dir.path(), &moved_dir).unwrap();
    let moved = Config {
        workspace_dir: moved_dir.clone(),
        ..Config::default()
    };
    let core = live::list_core_notifications(&moved, false, 10).unwrap();
    let ids: Vec<_> = core.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["e2", "e1"]);
    assert!(live::mark_core_notification_read(&moved, "e1").unwrap());
    assert_eq!(live::unread_core_notification_count(&moved).unwrap(), 1);
    crate::storage::local::forget(&moved_dir);
    // Put it back so the TempDir cleans up.
    std::fs::rename(&moved_dir, dir.path()).unwrap();
}
