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
}
