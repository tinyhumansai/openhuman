//! SQLite persistence for `IntegrationNotification` records.
//!
//! Uses a synchronous `rusqlite::Connection` opened per call, following the
//! same `with_connection` pattern as the cron domain.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};

use crate::openhuman::config::Config;

use super::types::{IntegrationNotification, NotificationSettings, NotificationStatus};

/// SQL schema applied on every `with_connection` call (idempotent).
const SCHEMA: &str = "
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS integration_notifications (
    id               TEXT PRIMARY KEY,
    provider         TEXT NOT NULL,
    account_id       TEXT,
    title            TEXT NOT NULL,
    body             TEXT NOT NULL,
    raw_payload      TEXT NOT NULL,
    importance_score REAL,
    triage_action    TEXT,
    triage_reason    TEXT,
    status           TEXT NOT NULL DEFAULT 'unread',
    received_at      TEXT NOT NULL,
    scored_at        TEXT
);
CREATE INDEX IF NOT EXISTS idx_integration_notifications_provider
    ON integration_notifications(provider);
CREATE INDEX IF NOT EXISTS idx_integration_notifications_status
    ON integration_notifications(status);
CREATE INDEX IF NOT EXISTS idx_integration_notifications_dedup
    ON integration_notifications(provider, account_id, title, body, received_at);

CREATE TABLE IF NOT EXISTS notification_settings (
    provider              TEXT PRIMARY KEY,
    enabled               INTEGER NOT NULL DEFAULT 1,
    importance_threshold  REAL NOT NULL DEFAULT 0.0,
    route_to_orchestrator INTEGER NOT NULL DEFAULT 1
);
";

/// Open (and migrate) the notifications DB, then call `f` with the live
/// connection. Mirrors the `with_connection` helper in `cron/store.rs`.
fn with_connection<T>(config: &Config, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    let db_path = config
        .workspace_dir
        .join("notifications")
        .join("notifications.db");

    tracing::trace!(
        path = %db_path.display(),
        "[notifications::store] opening DB connection"
    );

    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "[notifications::store] failed to create dir {}",
                parent.display()
            )
        })?;
    }

    let conn = Connection::open(&db_path).with_context(|| {
        format!(
            "[notifications::store] failed to open DB at {}",
            db_path.display()
        )
    })?;

    conn.execute_batch(SCHEMA)
        .context("[notifications::store] schema migration failed")?;

    tracing::trace!("[notifications::store] schema migration applied, running operation");
    f(&conn)
}

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Persist a new notification to the store.
pub fn insert(config: &Config, n: &IntegrationNotification) -> Result<()> {
    with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO integration_notifications
             (id, provider, account_id, title, body, raw_payload,
              importance_score, triage_action, triage_reason, status,
              received_at, scored_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                n.id,
                n.provider,
                n.account_id,
                n.title,
                n.body,
                n.raw_payload.to_string(),
                n.importance_score,
                n.triage_action,
                n.triage_reason,
                n.status.as_str(),
                n.received_at.to_rfc3339(),
                n.scored_at.map(|t| t.to_rfc3339()),
            ],
        )
        .context("[notifications::store] insert failed")?;
        Ok(())
    })
}

/// Atomically insert a notification unless a matching one arrived recently.
///
/// Returns `true` when inserted, `false` when skipped as duplicate.
pub fn insert_if_not_recent(config: &Config, n: &IntegrationNotification) -> Result<bool> {
    with_connection(config, |conn| {
        conn.execute_batch("BEGIN IMMEDIATE")
            .context("[notifications::store] begin insert_if_not_recent tx failed")?;

        let result: Result<bool> = (|| {
            let count: i64 = match n.account_id.as_deref() {
                Some(aid) => conn.query_row(
                    "SELECT COUNT(*) FROM integration_notifications
                     WHERE provider = ?1 AND account_id = ?2
                       AND title = ?3 AND body = ?4
                       AND unixepoch(received_at) >= unixepoch('now', '-60 seconds')",
                    params![&n.provider, aid, &n.title, &n.body],
                    |row| row.get(0),
                ),
                None => conn.query_row(
                    "SELECT COUNT(*) FROM integration_notifications
                     WHERE provider = ?1 AND account_id IS NULL
                       AND title = ?2 AND body = ?3
                       AND unixepoch(received_at) >= unixepoch('now', '-60 seconds')",
                    params![&n.provider, &n.title, &n.body],
                    |row| row.get(0),
                ),
            }
            .context("[notifications::store] insert_if_not_recent dedup query failed")?;

            if count > 0 {
                return Ok(false);
            }

            conn.execute(
                "INSERT INTO integration_notifications
                 (id, provider, account_id, title, body, raw_payload,
                  importance_score, triage_action, triage_reason, status,
                  received_at, scored_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    n.id,
                    n.provider,
                    n.account_id,
                    n.title,
                    n.body,
                    n.raw_payload.to_string(),
                    n.importance_score,
                    n.triage_action,
                    n.triage_reason,
                    n.status.as_str(),
                    n.received_at.to_rfc3339(),
                    n.scored_at.map(|t| t.to_rfc3339()),
                ],
            )
            .context("[notifications::store] insert_if_not_recent insert failed")?;

            Ok(true)
        })();

        if result.is_ok() {
            conn.execute_batch("COMMIT")
                .context("[notifications::store] commit insert_if_not_recent tx failed")?;
        } else if let Err(rollback_err) = conn.execute_batch("ROLLBACK") {
            tracing::warn!(
                error = %rollback_err,
                "[notifications::store] rollback insert_if_not_recent tx failed"
            );
        }

        result
    })
}

/// List notifications with optional filtering.
pub fn list(
    config: &Config,
    limit: usize,
    offset: usize,
    provider_filter: Option<&str>,
    min_score: Option<f32>,
) -> Result<Vec<IntegrationNotification>> {
    with_connection(config, |conn| {
        // Build a dynamic query instead of relying on nullable-aware WHERE
        // logic so the SQL stays readable for future contributors.
        let mut sql = String::from(
            "SELECT id, provider, account_id, title, body, raw_payload,
                    importance_score, triage_action, triage_reason, status,
                    received_at, scored_at
             FROM integration_notifications
             WHERE 1=1",
        );
        if provider_filter.is_some() {
            sql.push_str(" AND provider = ?1");
        }
        if min_score.is_some() {
            if provider_filter.is_some() {
                sql.push_str(" AND (importance_score IS NULL OR importance_score >= ?2)");
            } else {
                sql.push_str(" AND (importance_score IS NULL OR importance_score >= ?1)");
            }
        }
        sql.push_str(" ORDER BY received_at DESC");
        sql.push_str(&format!(" LIMIT {limit} OFFSET {offset}"));

        let mut stmt = conn
            .prepare(&sql)
            .context("[notifications::store] prepare list failed")?;

        let rows = match (provider_filter, min_score) {
            (Some(p), Some(s)) => stmt.query(params![p, s]),
            (Some(p), None) => stmt.query(params![p]),
            (None, Some(s)) => stmt.query(params![s]),
            (None, None) => stmt.query([]),
        }
        .context("[notifications::store] list query failed")?;

        rows_to_notifications(rows)
    })
}

/// Update triage scoring fields in-place.
pub fn update_triage(
    config: &Config,
    id: &str,
    score: f32,
    action: &str,
    reason: &str,
) -> Result<()> {
    with_connection(config, |conn| {
        let now = Utc::now().to_rfc3339();
        let updated = conn
            .execute(
                "UPDATE integration_notifications
             SET importance_score = ?1, triage_action = ?2, triage_reason = ?3, scored_at = ?4
             WHERE id = ?5",
                params![score, action, reason, now, id],
            )
            .context("[notifications::store] update_triage failed")?;
        if updated == 0 {
            // The row may have been deleted between ingest and scoring.
            // Surface it at warn level so orphaned triage runs don't fail
            // silently.
            tracing::warn!(
                id = %id,
                action = %action,
                "[notifications::store] update_triage matched no rows"
            );
        } else {
            tracing::debug!(
                id = %id,
                action = %action,
                score = score,
                "[notifications::store] update_triage applied"
            );
        }
        Ok(())
    })
}

/// Transition a notification from `unread` to `read`.
pub fn mark_read(config: &Config, id: &str) -> Result<()> {
    with_connection(config, |conn| {
        let updated = conn
            .execute(
                "UPDATE integration_notifications SET status = 'read' WHERE id = ?1",
                params![id],
            )
            .context("[notifications::store] mark_read failed")?;
        if updated == 0 {
            tracing::warn!(
                id = %id,
                "[notifications::store] mark_read matched no rows"
            );
        } else {
            tracing::debug!(id = %id, "[notifications::store] mark_read applied");
        }
        Ok(())
    })
}

/// Count unread notifications.
pub fn unread_count(config: &Config) -> Result<i64> {
    with_connection(config, |conn| {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM integration_notifications WHERE status = 'unread'",
                [],
                |row| row.get(0),
            )
            .context("[notifications::store] unread_count failed")?;
        Ok(count)
    })
}

/// Check whether a notification with identical content was received in the
/// last 60 seconds.
pub fn exists_recent(
    config: &Config,
    provider: &str,
    account_id: Option<&str>,
    title: &str,
    body: &str,
) -> Result<bool> {
    with_connection(config, |conn| {
        let count: i64 = match account_id {
            Some(aid) => conn.query_row(
                "SELECT COUNT(*) FROM integration_notifications
                 WHERE provider = ?1 AND account_id = ?2
                   AND title = ?3 AND body = ?4
                   AND unixepoch(received_at) >= unixepoch('now', '-60 seconds')",
                params![provider, aid, title, body],
                |row| row.get(0),
            ),
            None => conn.query_row(
                "SELECT COUNT(*) FROM integration_notifications
                 WHERE provider = ?1 AND account_id IS NULL
                   AND title = ?2 AND body = ?3
                   AND unixepoch(received_at) >= unixepoch('now', '-60 seconds')",
                params![provider, title, body],
                |row| row.get(0),
            ),
        }
        .context("[notifications::store] exists_recent query failed")?;
        Ok(count > 0)
    })
}

/// Transition a notification status to 'dismissed'.
///
/// Returns `true` when at least one row matched and was updated.
pub fn mark_dismissed(config: &Config, id: &str) -> Result<bool> {
    with_connection(config, |conn| {
        let updated = conn
            .execute(
                "UPDATE integration_notifications SET status = 'dismissed' WHERE id = ?1",
                params![id],
            )
            .context("[notification_intel] mark_dismissed failed")?;
        let matched = updated > 0;
        if !matched {
            tracing::warn!(id = %id, "[notification_intel] mark_dismissed matched no rows");
        } else {
            tracing::debug!(id = %id, "[notification_intel] mark_dismissed applied");
        }
        Ok(matched)
    })
}

/// Transition a notification status to 'acted'.
///
/// Returns `true` when at least one row matched and was updated.
pub fn mark_acted(config: &Config, id: &str) -> Result<bool> {
    with_connection(config, |conn| {
        let updated = conn
            .execute(
                "UPDATE integration_notifications SET status = 'acted' WHERE id = ?1",
                params![id],
            )
            .context("[notification_intel] mark_acted failed")?;
        let matched = updated > 0;
        if !matched {
            tracing::warn!(id = %id, "[notification_intel] mark_acted matched no rows");
        } else {
            tracing::debug!(id = %id, "[notification_intel] mark_acted applied");
        }
        Ok(matched)
    })
}

/// Return aggregate statistics for the notification intelligence pipeline.
pub fn stats(config: &Config) -> Result<super::types::NotificationStats> {
    use std::collections::HashMap;
    with_connection(config, |conn| {
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM integration_notifications", [], |r| {
                r.get(0)
            })
            .context("[notification_intel] stats total query failed")?;

        let unread: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM integration_notifications WHERE status = 'unread'",
                [],
                |r| r.get(0),
            )
            .context("[notification_intel] stats unread query failed")?;

        let unscored: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM integration_notifications WHERE importance_score IS NULL",
                [],
                |r| r.get(0),
            )
            .context("[notification_intel] stats unscored query failed")?;

        // Per-provider counts
        let mut by_provider = HashMap::new();
        {
            let mut stmt = conn
                .prepare(
                    "SELECT provider, COUNT(*) FROM integration_notifications GROUP BY provider",
                )
                .context("[notification_intel] stats by_provider prepare failed")?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
                .context("[notification_intel] stats by_provider query failed")?;
            for row in rows {
                let (provider, count) =
                    row.context("[notification_intel] stats by_provider row failed")?;
                by_provider.insert(provider, count);
            }
        }

        // Per-action counts (only where triage_action is set)
        let mut by_action = HashMap::new();
        {
            let mut stmt = conn
                .prepare(
                    "SELECT triage_action, COUNT(*) FROM integration_notifications \
                     WHERE triage_action IS NOT NULL GROUP BY triage_action",
                )
                .context("[notification_intel] stats by_action prepare failed")?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
                .context("[notification_intel] stats by_action query failed")?;
            for row in rows {
                let (action, count) =
                    row.context("[notification_intel] stats by_action row failed")?;
                by_action.insert(action, count);
            }
        }

        tracing::debug!(
            total = total,
            unread = unread,
            unscored = unscored,
            "[notification_intel] stats query completed"
        );

        Ok(super::types::NotificationStats {
            total,
            unread,
            unscored,
            by_provider,
            by_action,
        })
    })
}

/// Upsert provider-level notification settings.
pub fn upsert_settings(config: &Config, settings: &NotificationSettings) -> Result<()> {
    with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO notification_settings (provider, enabled, importance_threshold, route_to_orchestrator)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(provider) DO UPDATE SET
               enabled = excluded.enabled,
               importance_threshold = excluded.importance_threshold,
               route_to_orchestrator = excluded.route_to_orchestrator",
            params![
                settings.provider,
                if settings.enabled { 1 } else { 0 },
                settings.importance_threshold,
                if settings.route_to_orchestrator { 1 } else { 0 }
            ],
        )
        .context("[notifications::store] upsert_settings failed")?;
        Ok(())
    })
}

/// Read provider-level notification settings with defaults when missing.
pub fn get_settings(config: &Config, provider: &str) -> Result<NotificationSettings> {
    with_connection(config, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT provider, enabled, importance_threshold, route_to_orchestrator
                 FROM notification_settings
                 WHERE provider = ?1",
            )
            .context("[notifications::store] prepare get_settings failed")?;
        let mut rows = stmt
            .query(params![provider])
            .context("[notifications::store] get_settings query failed")?;
        if let Some(row) = rows
            .next()
            .context("[notifications::store] get_settings row failed")?
        {
            return Ok(NotificationSettings {
                provider: row.get(0)?,
                enabled: row.get::<_, i64>(1)? != 0,
                importance_threshold: row.get(2)?,
                route_to_orchestrator: row.get::<_, i64>(3)? != 0,
            });
        }
        Ok(NotificationSettings {
            provider: provider.to_string(),
            ..NotificationSettings::default()
        })
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Row conversion helpers
// ─────────────────────────────────────────────────────────────────────────────

fn rows_to_notifications(mut rows: rusqlite::Rows<'_>) -> Result<Vec<IntegrationNotification>> {
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .context("[notifications::store] row iteration failed")?
    {
        out.push(row_to_notification(row)?);
    }
    Ok(out)
}

fn row_to_notification(row: &rusqlite::Row<'_>) -> Result<IntegrationNotification> {
    let raw_payload_str: String = row.get(5)?;
    let raw_payload: serde_json::Value = serde_json::from_str(&raw_payload_str)
        .unwrap_or(serde_json::Value::String(raw_payload_str));

    let status_str: String = row.get(9)?;
    let status = match status_str.as_str() {
        "read" => NotificationStatus::Read,
        "acted" => NotificationStatus::Acted,
        "dismissed" => NotificationStatus::Dismissed,
        _ => NotificationStatus::Unread,
    };

    let received_at_str: String = row.get(10)?;
    let received_at: DateTime<Utc> = received_at_str.parse().unwrap_or_else(|e| {
        tracing::warn!(
            raw = %received_at_str,
            error = %e,
            "[notifications::store] invalid received_at, using now"
        );
        Utc::now()
    });

    let scored_at_str: Option<String> = row.get(11)?;
    let scored_at: Option<DateTime<Utc>> = scored_at_str.and_then(|s| match s.parse() {
        Ok(t) => Some(t),
        Err(e) => {
            tracing::warn!(
                raw = %s,
                error = %e,
                "[notifications::store] invalid scored_at, treating as unscored"
            );
            None
        }
    });

    Ok(IntegrationNotification {
        id: row.get(0)?,
        provider: row.get(1)?,
        account_id: row.get(2)?,
        title: row.get(3)?,
        body: row.get(4)?,
        raw_payload,
        importance_score: row.get(6)?,
        triage_action: row.get(7)?,
        triage_reason: row.get(8)?,
        status,
        received_at,
        scored_at,
    })
}

#[cfg(test)]
mod tests {
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
}
