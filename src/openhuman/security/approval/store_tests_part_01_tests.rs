use super::*;

#[test]
fn insert_then_list_returns_pending_row() {
    let (config, _dir) = test_config();
    insert_pending(&config, &sample("req-1", "sess-A"), "sess-A").unwrap();
    let rows = list_pending(&config).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].request_id, "req-1");
    assert_eq!(rows[0].tool_name, "composio");
}

#[test]
fn list_pending_returns_rows_from_every_session() {
    let (config, _dir) = test_config();
    insert_pending(&config, &sample("a", "sess-A"), "sess-A").unwrap();
    insert_pending(&config, &sample("b", "sess-B"), "sess-B").unwrap();
    let rows = list_pending(&config).unwrap();
    assert_eq!(
        rows.len(),
        2,
        "orphan rows from other sessions must remain visible"
    );
}

#[test]
fn decide_marks_row_and_excludes_from_pending_list() {
    let (config, _dir) = test_config();
    insert_pending(&config, &sample("req-9", "sess-A"), "sess-A").unwrap();
    let decided = decide(&config, "req-9", ApprovalDecision::ApproveOnce)
        .unwrap()
        .expect("decided row");
    assert_eq!(decided.request_id, "req-9");
    let rows = list_pending(&config).unwrap();
    assert!(rows.is_empty(), "decided rows should not appear in pending");
}

#[test]
fn decide_second_time_returns_none() {
    let (config, _dir) = test_config();
    insert_pending(&config, &sample("dupe", "sess-A"), "sess-A").unwrap();
    decide(&config, "dupe", ApprovalDecision::Deny).unwrap();
    let again = decide(&config, "dupe", ApprovalDecision::ApproveOnce).unwrap();
    assert!(again.is_none(), "second decide should be a no-op");
}

#[test]
fn decide_unknown_id_is_noop() {
    let (config, _dir) = test_config();
    let res = decide(&config, "never-existed", ApprovalDecision::Deny).unwrap();
    assert!(res.is_none());
}

#[test]
fn purge_session_removes_only_undecided_rows_for_session() {
    let (config, _dir) = test_config();
    insert_pending(&config, &sample("p1", "sess-A"), "sess-A").unwrap();
    insert_pending(&config, &sample("p2", "sess-A"), "sess-A").unwrap();
    insert_pending(&config, &sample("p3", "sess-B"), "sess-B").unwrap();
    decide(&config, "p2", ApprovalDecision::ApproveOnce).unwrap();
    let removed = purge_session(&config, "sess-A").unwrap();
    assert_eq!(removed, 1, "only undecided sess-A row should be purged");
    let remaining = list_pending(&config).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].request_id, "p3");
}

#[test]
fn get_decision_returns_none_until_decided_then_persisted_value() {
    // PR #2367 review: timeout-vs-decide race resolution in the
    // gate calls `get_decision` after a denied UPDATE no-ops.
    // Undecided rows and unknown ids must both return `None`,
    // and decided rows must round-trip the persisted decision.
    let (config, _dir) = test_config();
    assert!(get_decision(&config, "missing").unwrap().is_none());
    insert_pending(&config, &sample("race", "sess-A"), "sess-A").unwrap();
    assert!(
        get_decision(&config, "race").unwrap().is_none(),
        "undecided row reports no decision"
    );
    decide(&config, "race", ApprovalDecision::ApproveOnce).unwrap();
    assert_eq!(
        get_decision(&config, "race").unwrap(),
        Some(ApprovalDecision::ApproveOnce)
    );
}

#[test]
fn pending_row_survives_connection_close() {
    let (config, _dir) = test_config();
    insert_pending(&config, &sample("survives", "sess-A"), "sess-A").unwrap();
    let rows = list_pending(&config).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].request_id, "survives");
}

#[test]
fn record_execution_writes_terminal_audit_row_after_decide() {
    let (config, _dir) = test_config();
    insert_pending(&config, &sample("req-exec", "sess-A"), "sess-A").unwrap();
    // Before decide, record_execution must not patch the row —
    // a decided_at IS NOT NULL guard keeps the audit trail
    // monotonic (no "executed before approved").
    let early = record_execution(&config, "req-exec", ExecutionOutcome::Success, None).unwrap();
    assert!(!early, "record_execution before decide must be a no-op");
    let (exec_at, _, _) = read_execution_row(&config, "req-exec");
    assert!(exec_at.is_none());

    decide(&config, "req-exec", ApprovalDecision::ApproveOnce).unwrap();
    let ok = record_execution(&config, "req-exec", ExecutionOutcome::Success, None).unwrap();
    assert!(ok, "record_execution after decide must update the row");
    let (exec_at, outcome, error) = read_execution_row(&config, "req-exec");
    assert!(exec_at.is_some());
    assert_eq!(outcome.as_deref(), Some("success"));
    assert!(error.is_none());
}

#[test]
fn record_execution_persists_outcome_and_redacted_error() {
    let (config, _dir) = test_config();
    insert_pending(&config, &sample("req-fail", "sess-A"), "sess-A").unwrap();
    decide(&config, "req-fail", ApprovalDecision::ApproveOnce).unwrap();

    record_execution(
        &config,
        "req-fail",
        ExecutionOutcome::Failure,
        Some("backend returned 500"),
    )
    .unwrap();

    let (_, outcome, error) = read_execution_row(&config, "req-fail");
    assert_eq!(outcome.as_deref(), Some("failure"));
    assert_eq!(error.as_deref(), Some("backend returned 500"));
}

#[test]
fn record_execution_caps_long_error_messages() {
    let (config, _dir) = test_config();
    insert_pending(&config, &sample("req-long", "sess-A"), "sess-A").unwrap();
    decide(&config, "req-long", ApprovalDecision::ApproveOnce).unwrap();

    let huge = "x".repeat(2_000);
    record_execution(&config, "req-long", ExecutionOutcome::Failure, Some(&huge)).unwrap();

    let (_, _, error) = read_execution_row(&config, "req-long");
    let stored = error.expect("error stored");
    // 512-char cap is inclusive of the ellipsis marker
    // (CodeRabbit review on #2367) — anything longer would let
    // upstream crash dumps slowly fill the audit log.
    assert_eq!(
        stored.chars().count(),
        512,
        "truncated value must be exactly 512 chars (incl. ellipsis): {} chars",
        stored.chars().count()
    );
    assert!(stored.ends_with('…'));
}

#[test]
fn record_execution_redacts_secrets_in_error_message() {
    // PR #2367 review: upstream tool errors regularly echo back
    // the offending request including auth headers. The audit
    // row must persist the sanitized form so a leaked bearer
    // or API key never lands in the durable log.
    let (config, _dir) = test_config();
    insert_pending(&config, &sample("req-secret", "sess-A"), "sess-A").unwrap();
    decide(&config, "req-secret", ApprovalDecision::ApproveOnce).unwrap();

    let raw = "upstream 401: Authorization: Bearer sk-live-abcdef1234567890abcdef1234567890 \
         returned by sk-proj-FAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKE";
    record_execution(&config, "req-secret", ExecutionOutcome::Failure, Some(raw)).unwrap();

    let (_, _, error) = read_execution_row(&config, "req-secret");
    let stored = error.expect("error stored");
    assert!(
        !stored.contains("sk-live-abcdef1234567890abcdef1234567890"),
        "raw bearer token must not be persisted: {stored}"
    );
    assert!(
        !stored.contains("sk-proj-FAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKE"),
        "raw provider key must not be persisted: {stored}"
    );
    assert!(
        stored.contains("[REDACTED]"),
        "sanitizer must leave a redaction marker so audit reviewers see something was scrubbed: {stored}"
    );
}

#[test]
fn record_execution_is_idempotent_after_first_terminal_report_wins() {
    // CodeRabbit review on #2367: a late retry / cleanup path
    // must NOT rewrite the original audit row. The first
    // `record_execution` call wins; subsequent calls return
    // `false` and leave the row unchanged.
    let (config, _dir) = test_config();
    insert_pending(&config, &sample("req-idem", "sess-A"), "sess-A").unwrap();
    decide(&config, "req-idem", ApprovalDecision::ApproveOnce).unwrap();

    // First report: succeeds, row gets stamped.
    let first = record_execution(
        &config,
        "req-idem",
        ExecutionOutcome::Success,
        Some("ok-first"),
    )
    .unwrap();
    assert!(first);
    let (exec_at_1, outcome_1, error_1) = read_execution_row(&config, "req-idem");
    assert!(exec_at_1.is_some());
    assert_eq!(outcome_1.as_deref(), Some("success"));
    assert_eq!(error_1.as_deref(), Some("ok-first"));

    // Second report (e.g. a late retry that finally noticed the
    // outcome) must be a no-op and must NOT change the stored
    // outcome or timestamp.
    let second = record_execution(
        &config,
        "req-idem",
        ExecutionOutcome::Failure,
        Some("late-failure-noise"),
    )
    .unwrap();
    assert!(
        !second,
        "second record_execution must report no row updated"
    );

    let (exec_at_2, outcome_2, error_2) = read_execution_row(&config, "req-idem");
    assert_eq!(exec_at_2, exec_at_1, "executed_at must not change");
    assert_eq!(outcome_2.as_deref(), Some("success"));
    assert_eq!(error_2.as_deref(), Some("ok-first"));
}

#[test]
fn record_execution_unknown_id_is_safe_noop() {
    let (config, _dir) = test_config();
    let ok = record_execution(&config, "never-here", ExecutionOutcome::Success, None).unwrap();
    assert!(!ok, "unknown id must report no row updated");
}

#[test]
fn migrate_columns_is_idempotent_on_v1_databases() {
    // Simulate an older build by creating the v1 table shape
    // manually (no executed_at / execution_outcome / execution_error)
    // then opening the store via with_connection — the migration
    // must add the missing columns without losing existing rows.
    let dir = TempDir::new().unwrap();
    let workspace = dir.path().to_path_buf();
    let db_path = workspace.join("approval").join("approval.db");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE pending_approvals (
                request_id      TEXT PRIMARY KEY,
                tool_name       TEXT NOT NULL,
                action_summary  TEXT NOT NULL,
                args_redacted   TEXT NOT NULL,
                session_id      TEXT NOT NULL,
                created_at      TEXT NOT NULL,
                expires_at      TEXT,
                decided_at      TEXT,
                decision        TEXT
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO pending_approvals
                (request_id, tool_name, action_summary, args_redacted,
                 session_id, created_at)
             VALUES ('legacy', 'composio', 'legacy row', '{}', 'sess-X', ?1)",
            params![Utc::now().to_rfc3339()],
        )
        .unwrap();
    }
    let config = Config {
        workspace_dir: workspace,
        ..Config::default()
    };
    // First open triggers the migration; existing row survives.
    let rows = list_pending(&config).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].request_id, "legacy");
    // After migration, record_execution must work end-to-end.
    decide(&config, "legacy", ApprovalDecision::ApproveOnce).unwrap();
    assert!(record_execution(&config, "legacy", ExecutionOutcome::Success, None).unwrap());
    // Second open must be a no-op (migration is idempotent).
    let rows = list_pending(&config).unwrap();
    assert!(rows.is_empty(), "decided rows should not appear in pending");
}

#[test]
fn migrate_session_id_scrub_overwrites_legacy_values_and_bumps_user_version() {
    // Simulate an older build that wrote credential-shaped values
    // into `session_id`. After opening the store via
    // `with_connection`, every pre-existing session_id must be
    // overwritten with the redaction sentinel, and re-opening the
    // store must be a no-op (idempotent — guarded by user_version).
    let dir = TempDir::new().unwrap();
    let workspace = dir.path().to_path_buf();
    let db_path = workspace.join("approval").join("approval.db");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    // The bearer-shaped value below is a fixture, NOT a real
    // credential — picked to be obviously recognizable in any
    // diff if the scrub ever regresses.
    let bearer_shaped = "deadbeefcafebabe1234567890abcdef";
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute(
            "INSERT INTO pending_approvals
                (request_id, tool_name, action_summary, args_redacted,
                 session_id, created_at)
             VALUES ('legacy', 'composio', 'legacy row', '{}', ?1, ?2)",
            params![bearer_shaped, Utc::now().to_rfc3339()],
        )
        .unwrap();
        // Sanity-check: a fresh DB starts at user_version = 0.
        let v: i64 = conn
            .query_row("PRAGMA user_version", params![], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 0);
    }
    let config = Config {
        workspace_dir: workspace,
        ..Config::default()
    };
    // First open runs the scrub.
    let _ = list_pending(&config).unwrap();
    {
        let conn = Connection::open(&db_path).unwrap();
        let stored: String = conn
            .query_row(
                "SELECT session_id FROM pending_approvals WHERE request_id = 'legacy'",
                params![],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            stored, PRE_MIGRATION_SESSION_ID,
            "scrub must overwrite legacy session_id with the redaction sentinel"
        );
        let v: i64 = conn
            .query_row("PRAGMA user_version", params![], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 1, "user_version must be bumped to 1 after scrub");
    }
    // Second open must NOT touch already-scrubbed rows.
    let _ = list_pending(&config).unwrap();
    {
        let conn = Connection::open(&db_path).unwrap();
        let stored: String = conn
            .query_row(
                "SELECT session_id FROM pending_approvals WHERE request_id = 'legacy'",
                params![],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, PRE_MIGRATION_SESSION_ID);
    }
}

#[test]
fn list_pending_expires_stale_rows_before_returning() {
    let (config, _dir) = test_config();
    insert_pending(
        &config,
        &sample_with_expiry("expired", "sess-A", Some(Utc::now() - Duration::minutes(5))),
        "sess-A",
    )
    .unwrap();
    insert_pending(
        &config,
        &sample_with_expiry("active", "sess-A", Some(Utc::now() + Duration::minutes(5))),
        "sess-A",
    )
    .unwrap();

    let rows = list_pending(&config).unwrap();
    let ids: Vec<_> = rows.into_iter().map(|row| row.request_id).collect();
    assert_eq!(ids, vec!["active"]);

    let state = fetch_decision_state(&config, "expired").expect("expired row should persist");
    assert!(
        state.0.is_some(),
        "expired row should have decided_at recorded"
    );
    assert_eq!(state.1.as_deref(), Some("deny"));
}

#[test]
fn decide_on_expired_row_returns_none_and_keeps_terminal_audit_state() {
    let (config, _dir) = test_config();
    insert_pending(
        &config,
        &sample_with_expiry("late", "sess-A", Some(Utc::now() - Duration::minutes(1))),
        "sess-A",
    )
    .unwrap();

    let decided = decide(&config, "late", ApprovalDecision::ApproveOnce).unwrap();
    assert!(
        decided.is_none(),
        "late approvals should no longer be actionable"
    );

    let state = fetch_decision_state(&config, "late").expect("row should remain for audit");
    assert!(state.0.is_some());
    assert_eq!(state.1.as_deref(), Some("deny"));
}

#[test]
fn expire_stale_returns_number_of_rows_transitioned() {
    let (config, _dir) = test_config();
    insert_pending(
        &config,
        &sample_with_expiry("old-1", "sess-A", Some(Utc::now() - Duration::minutes(2))),
        "sess-A",
    )
    .unwrap();
    insert_pending(
        &config,
        &sample_with_expiry("old-2", "sess-B", Some(Utc::now() - Duration::minutes(1))),
        "sess-B",
    )
    .unwrap();
    insert_pending(
        &config,
        &sample_with_expiry("fresh", "sess-B", Some(Utc::now() + Duration::minutes(30))),
        "sess-B",
    )
    .unwrap();

    let expired = expire_stale(&config).unwrap();
    assert_eq!(expired, 2);

    let rows = list_pending(&config).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].request_id, "fresh");
}

#[test]
fn expire_stale_is_idempotent() {
    let (config, _dir) = test_config();
    insert_pending(
        &config,
        &sample_with_expiry("once", "sess-A", Some(Utc::now() - Duration::minutes(3))),
        "sess-A",
    )
    .unwrap();

    assert_eq!(expire_stale(&config).unwrap(), 1);
    assert_eq!(expire_stale(&config).unwrap(), 0);

    let state = fetch_decision_state(&config, "once").expect("row should remain recorded");
    assert!(state.0.is_some());
    assert_eq!(state.1.as_deref(), Some("deny"));
}

#[test]
fn expire_stale_leaves_non_expiring_rows_pending() {
    let (config, _dir) = test_config();
    insert_pending(
        &config,
        &sample_with_expiry("no-ttl", "sess-A", None),
        "sess-A",
    )
    .unwrap();

    assert_eq!(expire_stale(&config).unwrap(), 0);
    let rows = list_pending(&config).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].request_id, "no-ttl");

    let state = fetch_decision_state(&config, "no-ttl").expect("row should still exist");
    assert!(state.0.is_none());
    assert!(state.1.is_none());
}

#[test]
fn list_recent_decisions_returns_durable_audit_rows() {
    let (config, _dir) = test_config();
    insert_pending(&config, &sample("approved", "sess-A"), "sess-A").unwrap();
    insert_pending(&config, &sample("denied", "sess-B"), "sess-B").unwrap();
    decide(&config, "approved", ApprovalDecision::ApproveOnce).unwrap();
    decide(&config, "denied", ApprovalDecision::Deny).unwrap();

    let rows = list_recent_decisions(&config, 10).unwrap();

    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|row| {
        row.request_id == "approved" && row.decision == ApprovalDecision::ApproveOnce
    }));
    assert!(rows
        .iter()
        .any(|row| row.request_id == "denied" && row.decision == ApprovalDecision::Deny));
    assert!(
        rows.iter().all(|row| !row.tool_name.is_empty()),
        "audit rows should retain tool metadata"
    );
}

#[test]
fn list_recent_decisions_clamps_zero_limit_to_one() {
    let (config, _dir) = test_config();
    insert_pending(&config, &sample("one", "sess-A"), "sess-A").unwrap();
    insert_pending(&config, &sample("two", "sess-A"), "sess-A").unwrap();
    decide(&config, "one", ApprovalDecision::ApproveOnce).unwrap();
    decide(&config, "two", ApprovalDecision::Deny).unwrap();

    let rows = list_recent_decisions(&config, 0).unwrap();

    assert_eq!(rows.len(), 1);
}

#[test]
fn list_recent_decisions_rejects_unknown_decision_values() {
    let (config, _dir) = test_config();
    insert_pending(&config, &sample("corrupt-decision", "sess-A"), "sess-A").unwrap();
    with_connection(&config, |conn| {
        conn.execute(
            "UPDATE pending_approvals
             SET decided_at = ?1, decision = ?2
             WHERE request_id = ?3",
            params![Utc::now().to_rfc3339(), "maybe", "corrupt-decision"],
        )?;
        Ok(())
    })
    .unwrap();

    let err = list_recent_decisions(&config, 10).unwrap_err();

    assert!(
        err.to_string().contains("Invalid column type")
            || err.to_string().contains("unknown approval decision"),
        "unexpected error: {err}"
    );
}

#[test]
fn list_recent_decisions_rejects_invalid_audit_timestamps() {
    let (config, _dir) = test_config();
    insert_pending(&config, &sample("corrupt-time", "sess-A"), "sess-A").unwrap();
    with_connection(&config, |conn| {
        conn.execute(
            "UPDATE pending_approvals
             SET decided_at = ?1, decision = ?2
             WHERE request_id = ?3",
            params![
                "not-a-date",
                ApprovalDecision::Deny.as_str(),
                "corrupt-time"
            ],
        )?;
        Ok(())
    })
    .unwrap();

    let err = list_recent_decisions(&config, 10).unwrap_err();

    assert!(
        err.to_string().contains("Invalid column type")
            || err.to_string().contains("premature end of input"),
        "unexpected error: {err}"
    );
}

#[test]
fn insert_pending_round_trips_flow_source_context() {
    let (config, _dir) = test_config();
    let pending = flow_sample("flow-req-1", "flow-1", "run-1");
    insert_pending(&config, &pending, "sess-A").unwrap();

    let rows = list_pending(&config).unwrap();
    assert_eq!(rows.len(), 1);
    match &rows[0].source_context {
        Some(ApprovalSourceContext::Flow {
            flow_id, run_id, ..
        }) => {
            assert_eq!(flow_id, "flow-1");
            assert_eq!(run_id, "run-1");
        }
        other => panic!("expected Flow source_context, got {other:?}"),
    }
}

#[test]
fn insert_pending_without_source_context_round_trips_none() {
    let (config, _dir) = test_config();
    insert_pending(&config, &sample("chat-req-1", "sess-A"), "sess-A").unwrap();
    let rows = list_pending(&config).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0].source_context.is_none(),
        "chat-routed rows must not carry a source_context"
    );
}

#[test]
fn decide_preserves_source_context_on_the_returned_row() {
    let (config, _dir) = test_config();
    let pending = flow_sample("flow-req-2", "flow-2", "run-2");
    insert_pending(&config, &pending, "sess-A").unwrap();

    let decided = decide(
        &config,
        "flow-req-2",
        ApprovalDecision::ApproveAlwaysForFlow,
    )
    .unwrap()
    .expect("decided row");
    match decided.source_context {
        Some(ApprovalSourceContext::Flow {
            flow_id, run_id, ..
        }) => {
            assert_eq!(flow_id, "flow-2");
            assert_eq!(run_id, "run-2");
        }
        other => panic!("expected Flow source_context on decided row, got {other:?}"),
    }
}

#[test]
fn list_pending_for_flow_run_filters_to_the_matching_flow_and_run() {
    let (config, _dir) = test_config();
    insert_pending(&config, &flow_sample("a", "flow-1", "run-1"), "sess-A").unwrap();
    insert_pending(&config, &flow_sample("b", "flow-1", "run-2"), "sess-A").unwrap();
    insert_pending(&config, &flow_sample("c", "flow-2", "run-1"), "sess-A").unwrap();
    insert_pending(&config, &sample("d", "sess-A"), "sess-A").unwrap();

    let rows = list_pending_for_flow_run(&config, "flow-1", "run-1").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].request_id, "a");
}
