use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};

use crate::openhuman::config::Config;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS heartbeat_notification_state (
    dedupe_key TEXT PRIMARY KEY,
    event_fingerprint TEXT NOT NULL,
    source TEXT NOT NULL,
    category TEXT NOT NULL,
    stage TEXT NOT NULL,
    sent_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_heartbeat_notification_state_sent_at
    ON heartbeat_notification_state(sent_at);
";

pub struct SentMarker<'a> {
    pub dedupe_key: &'a str,
    pub event_fingerprint: &'a str,
    pub source: &'a str,
    pub category: &'a str,
    pub stage: &'a str,
    pub sent_at: DateTime<Utc>,
}

fn with_connection<T>(config: &Config, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    let db_path = config
        .workspace_dir
        .join("heartbeat")
        .join("heartbeat_state.db");

    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "[heartbeat::store] failed to create DB dir {}",
                parent.display()
            )
        })?;
    }

    let conn = Connection::open(&db_path).with_context(|| {
        format!(
            "[heartbeat::store] failed to open DB at {}",
            db_path.display()
        )
    })?;
    conn.execute_batch(SCHEMA)
        .context("[heartbeat::store] schema migration failed")?;

    f(&conn)
}

pub fn mark_sent(config: &Config, marker: &SentMarker<'_>) -> Result<bool> {
    with_connection(config, |conn| {
        let changed = conn
            .execute(
                "INSERT OR IGNORE INTO heartbeat_notification_state
                 (dedupe_key, event_fingerprint, source, category, stage, sent_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    marker.dedupe_key,
                    marker.event_fingerprint,
                    marker.source,
                    marker.category,
                    marker.stage,
                    marker.sent_at.to_rfc3339(),
                ],
            )
            .context("[heartbeat::store] mark_sent insert failed")?;

        Ok(changed > 0)
    })
}

pub fn prune_old(config: &Config, cutoff: DateTime<Utc>) -> Result<usize> {
    with_connection(config, |conn| {
        let changed = conn
            .execute(
                "DELETE FROM heartbeat_notification_state WHERE sent_at < ?1",
                params![cutoff.to_rfc3339()],
            )
            .context("[heartbeat::store] prune_old delete failed")?;
        Ok(changed)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config(tmp: &TempDir) -> Config {
        Config {
            workspace_dir: tmp.path().to_path_buf(),
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        }
    }

    #[test]
    fn mark_sent_dedupes_by_key() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let now = Utc::now();

        let first = mark_sent(
            &config,
            &SentMarker {
                dedupe_key: "a",
                event_fingerprint: "fp",
                source: "cron",
                category: "reminders",
                stage: "due",
                sent_at: now,
            },
        )
        .unwrap();

        let second = mark_sent(
            &config,
            &SentMarker {
                dedupe_key: "a",
                event_fingerprint: "fp",
                source: "cron",
                category: "reminders",
                stage: "due",
                sent_at: now,
            },
        )
        .unwrap();

        assert!(first);
        assert!(!second);
    }

    #[test]
    fn prune_old_removes_outdated_rows() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let now = Utc::now();

        mark_sent(
            &config,
            &SentMarker {
                dedupe_key: "old",
                event_fingerprint: "fp-old",
                source: "cron",
                category: "reminders",
                stage: "due",
                sent_at: now - chrono::Duration::days(30),
            },
        )
        .unwrap();

        mark_sent(
            &config,
            &SentMarker {
                dedupe_key: "new",
                event_fingerprint: "fp-new",
                source: "cron",
                category: "reminders",
                stage: "due",
                sent_at: now,
            },
        )
        .unwrap();

        let removed = prune_old(&config, now - chrono::Duration::days(14)).unwrap();
        assert_eq!(removed, 1);
    }
}
