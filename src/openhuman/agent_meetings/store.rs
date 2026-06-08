//! SQLite persistence for `MeetingSession` records.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::openhuman::config::Config;

use super::types::{AutoJoinSource, MeetingSession, MeetingSessionStatus};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meeting_sessions (
    id                 TEXT PRIMARY KEY,
    meet_url           TEXT NOT NULL,
    title              TEXT,
    calendar_event_id  TEXT,
    status             TEXT NOT NULL DEFAULT 'pending',
    source             TEXT NOT NULL DEFAULT 'manual',
    thread_id          TEXT,
    transcript_received INTEGER NOT NULL DEFAULT 0,
    summary_generated  INTEGER NOT NULL DEFAULT 0,
    created_at_ms      INTEGER NOT NULL,
    updated_at_ms      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_meeting_sessions_status
    ON meeting_sessions(status);
CREATE INDEX IF NOT EXISTS idx_meeting_sessions_meet_url
    ON meeting_sessions(meet_url);
";

fn with_connection<T>(config: &Config, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    let db_path = config.workspace_dir.join("meetings").join("meetings.db");

    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "[meetings::store] failed to create dir {}",
                parent.display()
            )
        })?;
    }

    let conn = Connection::open(&db_path).with_context(|| {
        format!(
            "[meetings::store] failed to open DB at {}",
            db_path.display()
        )
    })?;

    conn.execute_batch(SCHEMA)
        .context("[meetings::store] schema migration failed")?;

    f(&conn)
}

fn status_to_str(s: MeetingSessionStatus) -> &'static str {
    match s {
        MeetingSessionStatus::Pending => "pending",
        MeetingSessionStatus::Joined => "joined",
        MeetingSessionStatus::Active => "active",
        MeetingSessionStatus::Ended => "ended",
    }
}

fn str_to_status(s: &str) -> MeetingSessionStatus {
    match s {
        "joined" => MeetingSessionStatus::Joined,
        "active" => MeetingSessionStatus::Active,
        "ended" => MeetingSessionStatus::Ended,
        _ => MeetingSessionStatus::Pending,
    }
}

fn source_to_str(s: AutoJoinSource) -> &'static str {
    match s {
        AutoJoinSource::Calendar => "calendar",
        AutoJoinSource::Manual => "manual",
        AutoJoinSource::Api => "api",
    }
}

fn str_to_source(s: &str) -> AutoJoinSource {
    match s {
        "calendar" => AutoJoinSource::Calendar,
        "api" => AutoJoinSource::Api,
        _ => AutoJoinSource::Manual,
    }
}

fn row_to_session(row: &rusqlite::Row) -> rusqlite::Result<MeetingSession> {
    Ok(MeetingSession {
        id: row.get(0)?,
        meet_url: row.get(1)?,
        title: row.get(2)?,
        calendar_event_id: row.get(3)?,
        status: str_to_status(row.get::<_, String>(4)?.as_str()),
        source: str_to_source(row.get::<_, String>(5)?.as_str()),
        thread_id: row.get(6)?,
        transcript_received: row.get::<_, i64>(7)? != 0,
        summary_generated: row.get::<_, i64>(8)? != 0,
        created_at_ms: row.get::<_, i64>(9)? as u64,
        updated_at_ms: row.get::<_, i64>(10)? as u64,
    })
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Insert a new meeting session into the store. Fails if the `id` already exists.
pub fn create_session(config: &Config, session: &MeetingSession) -> Result<()> {
    with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO meeting_sessions
             (id, meet_url, title, calendar_event_id, status, source,
              thread_id, transcript_received, summary_generated,
              created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                session.id,
                session.meet_url,
                session.title,
                session.calendar_event_id,
                status_to_str(session.status),
                source_to_str(session.source),
                session.thread_id,
                session.transcript_received as i64,
                session.summary_generated as i64,
                session.created_at_ms as i64,
                session.updated_at_ms as i64,
            ],
        )?;
        Ok(())
    })
}

/// Retrieve a session by its unique ID. Returns `None` if not found.
pub fn get_session(config: &Config, id: &str) -> Result<Option<MeetingSession>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, meet_url, title, calendar_event_id, status, source,
                    thread_id, transcript_received, summary_generated,
                    created_at_ms, updated_at_ms
             FROM meeting_sessions WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], row_to_session)?;
        match rows.next() {
            Some(Ok(s)) => Ok(Some(s)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    })
}

/// Find the most recent session for a given meet URL. Returns `None` if none exist.
pub fn get_session_by_meet_url(config: &Config, url: &str) -> Result<Option<MeetingSession>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, meet_url, title, calendar_event_id, status, source,
                    thread_id, transcript_received, summary_generated,
                    created_at_ms, updated_at_ms
             FROM meeting_sessions WHERE meet_url = ?1
             ORDER BY created_at_ms DESC LIMIT 1",
        )?;
        let mut rows = stmt.query_map(params![url], row_to_session)?;
        match rows.next() {
            Some(Ok(s)) => Ok(Some(s)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    })
}

/// Transition a session to a new status and update `updated_at_ms`.
pub fn update_session_status(
    config: &Config,
    id: &str,
    status: MeetingSessionStatus,
    now_ms: u64,
) -> Result<()> {
    with_connection(config, |conn| {
        conn.execute(
            "UPDATE meeting_sessions SET status = ?1, updated_at_ms = ?2 WHERE id = ?3",
            params![status_to_str(status), now_ms as i64, id],
        )?;
        Ok(())
    })
}

/// Associate a conversation thread with an existing session.
pub fn set_session_thread_id(
    config: &Config,
    id: &str,
    thread_id: &str,
    now_ms: u64,
) -> Result<()> {
    with_connection(config, |conn| {
        conn.execute(
            "UPDATE meeting_sessions SET thread_id = ?1, updated_at_ms = ?2 WHERE id = ?3",
            params![thread_id, now_ms as i64, id],
        )?;
        Ok(())
    })
}

/// Return all sessions with status `pending`, `joined`, or `active` (most recent first).
pub fn list_active_sessions(config: &Config) -> Result<Vec<MeetingSession>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, meet_url, title, calendar_event_id, status, source,
                    thread_id, transcript_received, summary_generated,
                    created_at_ms, updated_at_ms
             FROM meeting_sessions WHERE status IN ('pending', 'joined', 'active')
             ORDER BY created_at_ms DESC",
        )?;
        let rows = stmt.query_map([], row_to_session)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    })
}

/// Flag that a transcript was received for this session.
pub fn mark_transcript_received(config: &Config, id: &str, now_ms: u64) -> Result<()> {
    with_connection(config, |conn| {
        conn.execute(
            "UPDATE meeting_sessions SET transcript_received = 1, updated_at_ms = ?1 WHERE id = ?2",
            params![now_ms as i64, id],
        )?;
        Ok(())
    })
}

/// Flag that a summary was generated for this session.
pub fn mark_summary_generated(config: &Config, id: &str, now_ms: u64) -> Result<()> {
    with_connection(config, |conn| {
        conn.execute(
            "UPDATE meeting_sessions SET summary_generated = 1, updated_at_ms = ?1 WHERE id = ?2",
            params![now_ms as i64, id],
        )?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::Config;
    use tempfile::TempDir;

    fn test_config() -> (Config, TempDir) {
        let dir = TempDir::new().unwrap();
        let mut config = Config::default();
        config.workspace_dir = dir.path().to_path_buf();
        (config, dir)
    }

    #[test]
    fn crud_cycle() {
        let (config, _dir) = test_config();

        let session = MeetingSession {
            id: "meet-001".into(),
            meet_url: "https://meet.google.com/abc-defg-hij".into(),
            title: Some("Daily standup".into()),
            calendar_event_id: Some("cal-xyz".into()),
            status: MeetingSessionStatus::Pending,
            source: AutoJoinSource::Calendar,
            thread_id: None,
            transcript_received: false,
            summary_generated: false,
            created_at_ms: 1000,
            updated_at_ms: 1000,
        };

        create_session(&config, &session).unwrap();

        let fetched = get_session(&config, "meet-001").unwrap().unwrap();
        assert_eq!(fetched.meet_url, session.meet_url);
        assert_eq!(fetched.status, MeetingSessionStatus::Pending);
        assert_eq!(fetched.source, AutoJoinSource::Calendar);
        assert!(!fetched.transcript_received);

        update_session_status(&config, "meet-001", MeetingSessionStatus::Joined, 2000).unwrap();
        let fetched = get_session(&config, "meet-001").unwrap().unwrap();
        assert_eq!(fetched.status, MeetingSessionStatus::Joined);
        assert_eq!(fetched.updated_at_ms, 2000);

        set_session_thread_id(&config, "meet-001", "thread-42", 3000).unwrap();
        let fetched = get_session(&config, "meet-001").unwrap().unwrap();
        assert_eq!(fetched.thread_id.as_deref(), Some("thread-42"));

        mark_transcript_received(&config, "meet-001", 4000).unwrap();
        let fetched = get_session(&config, "meet-001").unwrap().unwrap();
        assert!(fetched.transcript_received);

        mark_summary_generated(&config, "meet-001", 5000).unwrap();
        let fetched = get_session(&config, "meet-001").unwrap().unwrap();
        assert!(fetched.summary_generated);
    }

    #[test]
    fn get_by_meet_url() {
        let (config, _dir) = test_config();

        let session = MeetingSession {
            id: "meet-002".into(),
            meet_url: "https://meet.google.com/xyz".into(),
            title: None,
            calendar_event_id: None,
            status: MeetingSessionStatus::Active,
            source: AutoJoinSource::Manual,
            thread_id: None,
            transcript_received: false,
            summary_generated: false,
            created_at_ms: 100,
            updated_at_ms: 100,
        };
        create_session(&config, &session).unwrap();

        let found = get_session_by_meet_url(&config, "https://meet.google.com/xyz")
            .unwrap()
            .unwrap();
        assert_eq!(found.id, "meet-002");

        let none = get_session_by_meet_url(&config, "https://meet.google.com/nope").unwrap();
        assert!(none.is_none());
    }

    #[test]
    fn list_active_excludes_ended() {
        let (config, _dir) = test_config();

        for (id, status) in [
            ("m1", MeetingSessionStatus::Active),
            ("m2", MeetingSessionStatus::Ended),
            ("m3", MeetingSessionStatus::Pending),
        ] {
            let s = MeetingSession {
                id: id.into(),
                meet_url: format!("https://meet.google.com/{id}"),
                title: None,
                calendar_event_id: None,
                status,
                source: AutoJoinSource::Api,
                thread_id: None,
                transcript_received: false,
                summary_generated: false,
                created_at_ms: 1,
                updated_at_ms: 1,
            };
            create_session(&config, &s).unwrap();
        }

        let active = list_active_sessions(&config).unwrap();
        assert_eq!(active.len(), 2);
        assert!(active
            .iter()
            .all(|s| s.status != MeetingSessionStatus::Ended));
    }

    #[test]
    fn missing_session_returns_none() {
        let (config, _dir) = test_config();
        let result = get_session(&config, "nonexistent").unwrap();
        assert!(result.is_none());
    }
}
