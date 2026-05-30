#[cfg(test)]
mod tests {
    use crate::openhuman::subconscious::decision_log::DecisionLog;
    use crate::openhuman::subconscious::store;
    use crate::openhuman::subconscious::types::{
        EscalationPriority, EscalationStatus, TaskRecurrence, TaskSource, TickDecision,
    };

    #[test]
    fn sqlite_task_lifecycle_one_off() {
        let dir = tempfile::tempdir().unwrap();
        store::with_connection(dir.path(), |conn| {
            // Add a one-off task
            let task = store::add_task(
                conn,
                "Remind about meeting",
                TaskSource::User,
                TaskRecurrence::Once,
            )?;
            assert!(!task.completed);
            assert_eq!(task.recurrence, TaskRecurrence::Once);

            // Should be due immediately
            let due = store::due_tasks(conn, 9999999999.0)?;
            assert_eq!(due.len(), 1);

            // Execute and complete
            store::add_log_entry(
                conn,
                &task.id,
                1000.0,
                "act",
                Some("Reminded user"),
                Some(50),
            )?;
            store::mark_task_completed(conn, &task.id)?;

            // Should no longer be due
            let due = store::due_tasks(conn, 9999999999.0)?;
            assert_eq!(due.len(), 0);

            // Task still exists but completed
            let t = store::get_task(conn, &task.id)?;
            assert!(t.completed);

            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn sqlite_task_lifecycle_recurrent() {
        let dir = tempfile::tempdir().unwrap();
        store::with_connection(dir.path(), |conn| {
            let task = store::add_task(
                conn,
                "Check email",
                TaskSource::User,
                TaskRecurrence::Cron("0 8 * * *".into()),
            )?;

            // Execute and set next run
            let now = 1000.0;
            let next = 2000.0;
            store::add_log_entry(
                conn,
                &task.id,
                now,
                "act",
                Some("Checked 3 emails"),
                Some(200),
            )?;
            store::update_task_run_times(conn, &task.id, now, Some(next))?;

            // Not due yet (before next_run_at)
            let due = store::due_tasks(conn, 1500.0)?;
            assert_eq!(due.len(), 0);

            // Due after next_run_at
            let due = store::due_tasks(conn, 2500.0)?;
            assert_eq!(due.len(), 1);

            // Task should NOT be completed
            let t = store::get_task(conn, &task.id)?;
            assert!(!t.completed);

            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn escalation_approve_dismiss_flow() {
        let dir = tempfile::tempdir().unwrap();
        store::with_connection(dir.path(), |conn| {
            let task = store::add_task(
                conn,
                "Review deadline",
                TaskSource::User,
                TaskRecurrence::Once,
            )?;

            // Create escalation
            let esc = store::add_escalation(
                conn,
                &task.id,
                None,
                "Deadline conflict",
                "Two deadlines on the same day",
                &EscalationPriority::Important,
            )?;
            assert_eq!(esc.status, EscalationStatus::Pending);
            assert_eq!(store::pending_escalation_count(conn)?, 1);

            // Approve
            store::resolve_escalation(conn, &esc.id, &EscalationStatus::Approved)?;
            let resolved = store::get_escalation(conn, &esc.id)?;
            assert_eq!(resolved.status, EscalationStatus::Approved);
            assert!(resolved.resolved_at.is_some());
            assert_eq!(store::pending_escalation_count(conn)?, 0);

            // Create another and dismiss
            let esc2 = store::add_escalation(
                conn,
                &task.id,
                None,
                "Budget warning",
                "Monthly spend at 90%",
                &EscalationPriority::Normal,
            )?;
            store::resolve_escalation(conn, &esc2.id, &EscalationStatus::Dismissed)?;
            let dismissed = store::get_escalation(conn, &esc2.id)?;
            assert_eq!(dismissed.status, EscalationStatus::Dismissed);

            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn execution_log_tracks_history() {
        let dir = tempfile::tempdir().unwrap();
        store::with_connection(dir.path(), |conn| {
            let task = store::add_task(
                conn,
                "Check health",
                TaskSource::System,
                TaskRecurrence::Pending,
            )?;

            store::add_log_entry(conn, &task.id, 1000.0, "noop", Some("All healthy"), None)?;
            store::add_log_entry(
                conn,
                &task.id,
                2000.0,
                "act",
                Some("Restarted skill"),
                Some(500),
            )?;
            store::add_log_entry(conn, &task.id, 3000.0, "noop", Some("All healthy"), None)?;

            let entries = store::list_log_entries(conn, Some(&task.id), 10)?;
            assert_eq!(entries.len(), 3);
            // Most recent first
            assert_eq!(entries[0].tick_at, 3000.0);
            assert_eq!(entries[1].decision, "act");

            // Global log
            let all = store::list_log_entries(conn, None, 2)?;
            assert_eq!(all.len(), 2); // limited to 2

            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn decision_log_dedup_still_works() {
        let mut log = DecisionLog::new();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        log.record(
            now,
            TickDecision::Escalate,
            "deadline email",
            vec!["doc-1".into()],
        );

        // doc-1 should be filtered as already surfaced
        let unsurfaced = log.filter_unsurfaced(&["doc-1".into(), "doc-2".into()]);
        assert!(!unsurfaced.contains(&"doc-1".to_string()));
        assert!(unsurfaced.contains(&"doc-2".to_string()));

        // Acknowledge doc-1
        log.mark_acknowledged(&["doc-1".into()]);
        assert!(!log.was_already_surfaced(&["doc-1".into()]));
    }

    #[test]
    fn seed_then_query_tasks() {
        let dir = tempfile::tempdir().unwrap();

        store::with_connection(dir.path(), |conn| {
            let count = store::seed_default_tasks(conn)?;
            assert_eq!(count, 3);

            let tasks = store::list_tasks(conn, true)?;
            assert_eq!(tasks.len(), 3);
            assert!(tasks.iter().all(|t| t.source == TaskSource::System));
            assert!(tasks
                .iter()
                .all(|t| t.recurrence == TaskRecurrence::Pending));

            // All should be due (no next_run_at set)
            let due = store::due_tasks(conn, 9999999999.0)?;
            assert_eq!(due.len(), 3);

            Ok(())
        })
        .unwrap();
    }

    /// Regression test for the "empty task list on fresh install" bug.
    ///
    /// The core server's startup path calls `get_or_init_engine()` to
    /// eagerly construct a `SubconsciousEngine`, relying on the constructor
    /// to seed the 3 default system tasks. This test locks in that
    /// invariant: constructing the engine alone — with no tick, no
    /// trigger RPC, and no explicit seed call — must leave the 3 defaults
    /// in the SQLite store.
    #[test]
    fn engine_construction_seeds_default_tasks() {
        use crate::openhuman::config::HeartbeatConfig;
        use crate::openhuman::subconscious::SubconsciousEngine;

        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().to_path_buf();

        // Construct the engine via the same path the core server uses at
        // startup. Memory client is not required for seeding.
        let _engine = SubconsciousEngine::from_heartbeat_config(
            &HeartbeatConfig::default(),
            workspace.clone(),
            None,
        );

        // The 3 default system tasks must now exist in the store.
        store::with_connection(&workspace, |conn| {
            let tasks = store::list_tasks(conn, false)?;
            assert_eq!(
                tasks.len(),
                3,
                "engine construction must seed the 3 default system tasks"
            );
            assert!(tasks.iter().all(|t| t.source == TaskSource::System));
            assert!(tasks
                .iter()
                .all(|t| t.recurrence == TaskRecurrence::Pending));

            Ok(())
        })
        .unwrap();

        // Reconstructing the engine on the same workspace must not
        // duplicate the defaults — seed_default_tasks is idempotent.

        let _engine2 = SubconsciousEngine::from_heartbeat_config(
            &HeartbeatConfig::default(),
            workspace.clone(),
            None,
        );

        store::with_connection(&workspace, |conn| {
            let tasks = store::list_tasks(conn, false)?;
            assert_eq!(
                tasks.len(),
                3,
                "repeat engine construction must not duplicate default tasks"
            );
            Ok(())
        })
        .unwrap();
    }
}
