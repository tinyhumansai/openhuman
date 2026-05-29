use super::*;

fn test_conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(SCHEMA_DDL).unwrap();
    conn
}

#[test]
fn crud_tasks() {
    let conn = test_conn();
    let task = add_task(&conn, "Check email", TaskSource::User, TaskRecurrence::Once).unwrap();
    assert_eq!(task.title, "Check email");
    assert!(!task.completed);

    let fetched = get_task(&conn, &task.id).unwrap();
    assert_eq!(fetched.title, "Check email");

    let all = list_tasks(&conn, false).unwrap();
    assert_eq!(all.len(), 1);

    update_task(
        &conn,
        &task.id,
        &TaskPatch {
            title: Some("Check Gmail".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let updated = get_task(&conn, &task.id).unwrap();
    assert_eq!(updated.title, "Check Gmail");

    mark_task_completed(&conn, &task.id).unwrap();
    let done = get_task(&conn, &task.id).unwrap();
    assert!(done.completed);

    remove_task(&conn, &task.id).unwrap();
    assert!(get_task(&conn, &task.id).is_err());
}

#[test]
fn due_tasks_filters_correctly() {
    let conn = test_conn();
    let now = now_secs();

    // Task with no next_run_at — should be due
    add_task(
        &conn,
        "No schedule",
        TaskSource::User,
        TaskRecurrence::Pending,
    )
    .unwrap();

    // Task with future next_run_at — should NOT be due
    let future_task =
        add_task(&conn, "Future task", TaskSource::User, TaskRecurrence::Once).unwrap();
    update_task_run_times(&conn, &future_task.id, now, Some(now + 3600.0)).unwrap();

    // Task with past next_run_at — should be due
    let past_task = add_task(&conn, "Past due", TaskSource::User, TaskRecurrence::Once).unwrap();
    update_task_run_times(&conn, &past_task.id, now - 7200.0, Some(now - 3600.0)).unwrap();

    let due = due_tasks(&conn, now).unwrap();
    assert_eq!(due.len(), 2); // "No schedule" + "Past due"
    assert!(due.iter().any(|t| t.title == "No schedule"));
    assert!(due.iter().any(|t| t.title == "Past due"));
    assert!(!due.iter().any(|t| t.title == "Future task"));
}

#[test]
fn crud_log_entries() {
    let conn = test_conn();
    let task = add_task(&conn, "Test", TaskSource::User, TaskRecurrence::Once).unwrap();
    let now = now_secs();

    let entry = add_log_entry(
        &conn,
        &task.id,
        now,
        "act",
        Some("Did the thing"),
        Some(150),
    )
    .unwrap();
    assert_eq!(entry.decision, "act");

    let entries = list_log_entries(&conn, Some(&task.id), 10).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].result.as_deref(), Some("Did the thing"));

    let all_entries = list_log_entries(&conn, None, 10).unwrap();
    assert_eq!(all_entries.len(), 1);
}

#[test]
fn crud_escalations() {
    let conn = test_conn();
    let task = add_task(&conn, "Test", TaskSource::User, TaskRecurrence::Once).unwrap();

    let esc = add_escalation(
        &conn,
        &task.id,
        None,
        "Deadline moved",
        "The API deadline was moved to tomorrow",
        &EscalationPriority::Critical,
    )
    .unwrap();
    assert_eq!(esc.status, EscalationStatus::Pending);

    let pending = list_escalations(&conn, Some(&EscalationStatus::Pending)).unwrap();
    assert_eq!(pending.len(), 1);

    assert_eq!(pending_escalation_count(&conn).unwrap(), 1);

    resolve_escalation(&conn, &esc.id, &EscalationStatus::Approved).unwrap();
    let resolved = get_escalation(&conn, &esc.id).unwrap();
    assert_eq!(resolved.status, EscalationStatus::Approved);
    assert!(resolved.resolved_at.is_some());

    assert_eq!(pending_escalation_count(&conn).unwrap(), 0);
}

#[test]
fn seed_default_tasks_creates_system_tasks() {
    let conn = test_conn();

    let count = seed_default_tasks(&conn).unwrap();
    assert_eq!(count, DEFAULT_SYSTEM_TASKS.len());

    // Second seed should not duplicate
    let count2 = seed_default_tasks(&conn).unwrap();
    assert_eq!(count2, 0);

    let tasks = list_tasks(&conn, false).unwrap();
    assert_eq!(tasks.len(), DEFAULT_SYSTEM_TASKS.len());
    assert!(tasks.iter().all(|t| t.source == TaskSource::System));
}

#[test]
fn recurrence_roundtrip() {
    assert_eq!(
        string_to_recurrence(&recurrence_to_string(&TaskRecurrence::Once)),
        TaskRecurrence::Once
    );
    assert_eq!(
        string_to_recurrence(&recurrence_to_string(&TaskRecurrence::Pending)),
        TaskRecurrence::Pending
    );
    assert_eq!(
        string_to_recurrence(&recurrence_to_string(&TaskRecurrence::Cron(
            "0 8 * * *".into()
        ))),
        TaskRecurrence::Cron("0 8 * * *".into())
    );
}

// ── DDL resilience: classifier + retry happy path ──────────────
//
// These guards back Sentry TAURI-RUST-A: the production failure is
// `Connection::open` + `execute_batch(SCHEMA_DDL)` racing against
// another in-process connection that holds the write lock. With
// `BUSY_TIMEOUT` set and the application-level retry loop in place,
// the race resolves on its own; without them the first attempt
// returns `SQLITE_BUSY` and the user sees "failed to run subconscious
// schema DDL" in Sentry with no further context.

#[test]
fn is_sqlite_busy_matches_database_busy_code() {
    let raw = rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error {
            code: rusqlite::ErrorCode::DatabaseBusy,
            extended_code: 5, // SQLITE_BUSY
        },
        Some("database is locked".into()),
    );
    let err = anyhow::Error::from(raw);
    assert!(is_sqlite_busy(&err));
}

#[test]
fn is_sqlite_busy_matches_database_locked_code() {
    let raw = rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error {
            code: rusqlite::ErrorCode::DatabaseLocked,
            extended_code: 6, // SQLITE_LOCKED
        },
        Some("database table is locked".into()),
    );
    let err = anyhow::Error::from(raw);
    assert!(is_sqlite_busy(&err));
}

#[test]
fn is_sqlite_busy_does_not_match_constraint_violation() {
    let raw = rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error {
            code: rusqlite::ErrorCode::ConstraintViolation,
            extended_code: 19,
        },
        Some("UNIQUE constraint failed".into()),
    );
    let err = anyhow::Error::from(raw);
    assert!(!is_sqlite_busy(&err));
}

#[test]
fn is_sqlite_busy_does_not_match_schema_syntax_error() {
    // A genuine bug in `SCHEMA_DDL` (e.g. typo in CREATE TABLE) would
    // surface as a `SqliteFailure(Unknown, ...)` with "syntax error"
    // in the message — retrying just delays the same failure, so the
    // classifier must reject it. Use Unknown + non-busy message.
    let raw = rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error {
            code: rusqlite::ErrorCode::Unknown,
            extended_code: 1,
        },
        Some("near \"FOO\": syntax error".into()),
    );
    let err = anyhow::Error::from(raw);
    assert!(!is_sqlite_busy(&err));
}

#[test]
fn is_sqlite_busy_matches_through_context_layers() {
    // The production failure shape: a rusqlite error wrapped under
    // `with_context("failed to run subconscious schema DDL")` —
    // exactly what `open_and_initialize` produces. Downcast must
    // still find the rusqlite root.
    let raw = rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error {
            code: rusqlite::ErrorCode::DatabaseBusy,
            extended_code: 5,
        },
        Some("database is locked".into()),
    );
    let wrapped: anyhow::Result<()> = Err(anyhow::Error::from(raw))
        .with_context(|| "failed to run subconscious schema DDL".to_string());
    let err = wrapped.unwrap_err();
    assert!(is_sqlite_busy(&err));
}

#[test]
fn is_sqlite_busy_text_fallback_when_downcast_misses() {
    // If a future refactor stringifies the rusqlite error before
    // wrapping (e.g. via anyhow!("{e}")), the downcast misses but
    // the chain-formatter text still preserves "database is locked".
    let err = anyhow::anyhow!("failed to run subconscious schema DDL: database is locked");
    assert!(is_sqlite_busy(&err));
}

#[test]
fn with_connection_resolves_external_write_contention() {
    use std::sync::mpsc;
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().to_path_buf();

    // First call: prime the DB so the file exists and the schema is
    // initialized. Subsequent calls take the fast path.
    with_connection(&workspace, |conn| {
        add_task(conn, "primer", TaskSource::User, TaskRecurrence::Once)?;
        Ok(())
    })
    .expect("prime DB");

    // Hold an EXCLUSIVE write lock for ~250 ms in a side thread.
    // The DDL loop in `open_and_initialize` re-runs PRAGMA journal_mode
    // and CREATE TABLE IF NOT EXISTS — both are no-ops on an already
    // initialized DB but still acquire the write lock briefly, which
    // races against the held lock. The application-level retry
    // (100 / 300 / 900 ms) plus the 5 s `busy_timeout` must absorb
    // this and let the second `with_connection` succeed.
    let db_path = workspace.join("subconscious").join("subconscious.db");
    let (lock_ready_tx, lock_ready_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let blocker = std::thread::spawn(move || {
        let conn = rusqlite::Connection::open(&db_path).expect("open blocker conn");
        conn.busy_timeout(std::time::Duration::from_millis(100))
            .expect("blocker busy_timeout");
        let tx = conn
            .unchecked_transaction()
            .expect("begin blocker transaction");
        // Force write-lock acquisition immediately.
        tx.execute(
            "INSERT INTO subconscious_tasks \
             (id, title, source, recurrence, created_at) \
             VALUES ('blocker', 'blocker', 'user', 'pending', 0.0)",
            [],
        )
        .expect("blocker insert");
        lock_ready_tx.send(()).expect("signal lock acquired");
        // Wait for the main thread to start contending, then a touch
        // longer so the first one or two retries collide.
        release_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("release signal");
        std::thread::sleep(std::time::Duration::from_millis(50));
        tx.rollback().expect("rollback blocker txn");
    });

    // Wait until the blocker actually holds the write lock.
    lock_ready_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("blocker never acquired lock");

    // Contender: should retry through the busy window and succeed
    // once the blocker rolls back. We release the blocker after
    // ~250 ms so the second / third retry attempt lands in the
    // unlocked window.
    let release_tx_for_timer = release_tx.clone();
    let timer = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(250));
        let _ = release_tx_for_timer.send(());
    });

    let result = with_connection(&workspace, |conn| {
        // Confirm we can issue a real query through the contended
        // connection — proves the open + DDL completed cleanly.
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM subconscious_tasks", [], |row| {
                row.get(0)
            })
            .unwrap_or(-1);
        Ok(count)
    });

    timer.join().expect("timer thread panicked");
    blocker.join().expect("blocker thread panicked");

    let count = result.expect("contended with_connection must succeed via retry");
    // Primer row is "primer"; blocker's INSERT was rolled back, so
    // the count should be exactly 1.
    assert_eq!(
        count, 1,
        "expected only the primer row after blocker rollback, got {count}"
    );
}
