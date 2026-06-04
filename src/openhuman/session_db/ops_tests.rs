use super::*;
use crate::openhuman::session_db::store::with_memory_connection;
use crate::openhuman::session_db::types::SessionSearchParams;

fn insert_test_session(conn: &Connection, id: &str, agent_id: &str, key: &str) {
    let now = Utc::now();
    conn.execute(
        "INSERT INTO sessions (
            id, agent_definition_id, agent_definition_name, session_key,
            status, started_at
         ) VALUES (?1, ?2, ?3, ?4, 'running', ?5)",
        params![id, agent_id, agent_id, key, now.to_rfc3339()],
    )
    .unwrap();
    index_fts_session(conn, id, agent_id).unwrap();
}

fn insert_test_session_with_parent(
    conn: &Connection,
    id: &str,
    agent_id: &str,
    key: &str,
    parent_id: &str,
) {
    let now = Utc::now();
    conn.execute(
        "INSERT INTO sessions (
            id, agent_definition_id, agent_definition_name, session_key,
            parent_session_id, status, started_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6)",
        params![id, agent_id, agent_id, key, parent_id, now.to_rfc3339()],
    )
    .unwrap();
    index_fts_session(conn, id, agent_id).unwrap();
}

#[test]
fn map_session_row_roundtrip() {
    with_memory_connection(|conn| {
        insert_test_session(conn, "sess-1", "orchestrator", "1700000000_orchestrator");

        let mut stmt = conn.prepare(
            "SELECT id, agent_definition_id, agent_definition_name, session_key,
                    parent_session_id, thread_id, source_channel, status, model,
                    turn_count, input_tokens, output_tokens, cached_input_tokens,
                    cost_usd, transcript_path, started_at, ended_at
             FROM sessions WHERE id = 'sess-1'",
        )?;
        let session = stmt.query_row([], map_session_row)?;

        assert_eq!(session.id, "sess-1");
        assert_eq!(session.agent_definition_id, "orchestrator");
        assert_eq!(session.session_key, "1700000000_orchestrator");
        assert_eq!(session.status, SessionStatus::Running);
        assert!(session.parent_session_id.is_none());
        assert!(session.ended_at.is_none());
        Ok(())
    })
    .unwrap();
}

#[test]
fn search_by_agent_id() {
    with_memory_connection(|conn| {
        insert_test_session(conn, "a1", "orchestrator", "key1");
        insert_test_session(conn, "a2", "researcher", "key2");
        insert_test_session(conn, "a3", "orchestrator", "key3");

        let params = SessionSearchParams {
            agent_id: Some("orchestrator".to_string()),
            ..Default::default()
        };

        let result = search_sessions_inner(conn, &params)?;
        assert_eq!(result.total, 2);
        assert_eq!(result.sessions.len(), 2);
        assert!(result
            .sessions
            .iter()
            .all(|s| s.agent_definition_id == "orchestrator"));
        Ok(())
    })
    .unwrap();
}

#[test]
fn search_by_fts_query() {
    with_memory_connection(|conn| {
        insert_test_session(conn, "b1", "orchestrator", "key1");
        insert_test_session(conn, "b2", "researcher", "key2");

        conn.execute(
            "INSERT INTO session_messages (session_id, role, content, created_at)
             VALUES ('b1', 'user', 'Fix the login bug in authentication', ?1)",
            params![Utc::now().to_rfc3339()],
        )?;
        index_fts_content(conn, "b1", "Fix the login bug in authentication")?;

        conn.execute(
            "INSERT INTO session_messages (session_id, role, content, created_at)
             VALUES ('b2', 'user', 'Deploy the new feature to production', ?1)",
            params![Utc::now().to_rfc3339()],
        )?;
        index_fts_content(conn, "b2", "Deploy the new feature to production")?;

        let params = SessionSearchParams {
            query: Some("login".to_string()),
            ..Default::default()
        };

        let result = search_sessions_inner(conn, &params)?;
        assert_eq!(result.total, 1);
        assert_eq!(result.sessions[0].id, "b1");
        Ok(())
    })
    .unwrap();
}

#[test]
fn search_by_tool_name() {
    with_memory_connection(|conn| {
        insert_test_session(conn, "c1", "orchestrator", "key1");
        insert_test_session(conn, "c2", "researcher", "key2");

        conn.execute(
            "INSERT INTO session_tool_calls (session_id, tool_name, status, created_at)
             VALUES ('c1', 'shell', 'ok', ?1)",
            params![Utc::now().to_rfc3339()],
        )?;
        conn.execute(
            "INSERT INTO session_tool_calls (session_id, tool_name, status, created_at)
             VALUES ('c2', 'file_read', 'ok', ?1)",
            params![Utc::now().to_rfc3339()],
        )?;

        let params = SessionSearchParams {
            tool_name: Some("shell".to_string()),
            ..Default::default()
        };

        let result = search_sessions_inner(conn, &params)?;
        assert_eq!(result.total, 1);
        assert_eq!(result.sessions[0].id, "c1");
        Ok(())
    })
    .unwrap();
}

#[test]
fn search_by_parent_session() {
    with_memory_connection(|conn| {
        insert_test_session(conn, "parent-1", "orchestrator", "key1");
        insert_test_session_with_parent(conn, "child-1", "researcher", "key2", "parent-1");
        insert_test_session_with_parent(conn, "child-2", "coder", "key3", "parent-1");
        insert_test_session(conn, "unrelated", "other", "key4");

        let params = SessionSearchParams {
            parent_session_id: Some("parent-1".to_string()),
            ..Default::default()
        };

        let result = search_sessions_inner(conn, &params)?;
        assert_eq!(result.total, 2);
        Ok(())
    })
    .unwrap();
}

#[test]
fn search_pagination() {
    with_memory_connection(|conn| {
        for i in 0..10 {
            insert_test_session(conn, &format!("p{i}"), "agent", &format!("key{i}"));
        }

        let params = SessionSearchParams {
            limit: Some(3),
            offset: Some(0),
            ..Default::default()
        };
        let result = search_sessions_inner(conn, &params)?;
        assert_eq!(result.total, 10);
        assert_eq!(result.sessions.len(), 3);

        let params2 = SessionSearchParams {
            limit: Some(3),
            offset: Some(3),
            ..Default::default()
        };
        let result2 = search_sessions_inner(conn, &params2)?;
        assert_eq!(result2.total, 10);
        assert_eq!(result2.sessions.len(), 3);
        assert_ne!(result.sessions[0].id, result2.sessions[0].id);

        Ok(())
    })
    .unwrap();
}

#[test]
fn search_empty_results() {
    with_memory_connection(|conn| {
        let params = SessionSearchParams {
            agent_id: Some("nonexistent".to_string()),
            ..Default::default()
        };
        let result = search_sessions_inner(conn, &params)?;
        assert_eq!(result.total, 0);
        assert!(result.sessions.is_empty());
        Ok(())
    })
    .unwrap();
}

#[test]
fn tool_output_truncation() {
    with_memory_connection(|conn| {
        let session_id = "trunc-sess";
        insert_test_session(conn, session_id, "agent", "key");

        let large_output = "x".repeat(MAX_TOOL_OUTPUT_BYTES + 1000);
        let bounded = if large_output.len() <= MAX_TOOL_OUTPUT_BYTES {
            large_output.clone()
        } else {
            let mut cutoff = MAX_TOOL_OUTPUT_BYTES;
            while cutoff > 0 && !large_output.is_char_boundary(cutoff) {
                cutoff -= 1;
            }
            let mut truncated = large_output[..cutoff].to_string();
            truncated.push_str("\n...[truncated]");
            truncated
        };

        conn.execute(
            "INSERT INTO session_tool_calls (session_id, tool_name, tool_output, status, created_at)
             VALUES (?1, 'test', ?2, 'ok', ?3)",
            params![session_id, bounded, Utc::now().to_rfc3339()],
        )?;

        let stored: String = conn.query_row(
            "SELECT tool_output FROM session_tool_calls WHERE session_id = ?1",
            params![session_id],
            |r| r.get(0),
        )?;
        assert!(stored.len() <= MAX_TOOL_OUTPUT_BYTES + 20);
        assert!(stored.ends_with("[truncated]"));

        Ok(())
    })
    .unwrap();
}

#[test]
fn mark_interrupted_updates_running() {
    with_memory_connection(|conn| {
        insert_test_session(conn, "run1", "agent", "key1");
        insert_test_session(conn, "run2", "agent", "key2");
        conn.execute(
            "UPDATE sessions SET status = 'completed' WHERE id = 'run2'",
            [],
        )?;

        let now = Utc::now();
        let changed = conn.execute(
            "UPDATE sessions SET status = 'interrupted', ended_at = ?1
             WHERE status = 'running'",
            params![now.to_rfc3339()],
        )?;
        assert_eq!(changed, 1);

        let status: String =
            conn.query_row("SELECT status FROM sessions WHERE id = 'run1'", [], |r| {
                r.get(0)
            })?;
        assert_eq!(status, "interrupted");

        let status2: String =
            conn.query_row("SELECT status FROM sessions WHERE id = 'run2'", [], |r| {
                r.get(0)
            })?;
        assert_eq!(status2, "completed");

        Ok(())
    })
    .unwrap();
}

#[test]
fn session_end_updates_cost_fields() {
    with_memory_connection(|conn| {
        insert_test_session(conn, "cost-sess", "agent", "key");

        let now = Utc::now();
        conn.execute(
            "UPDATE sessions SET
                status = 'completed', turn_count = 5, input_tokens = 10000,
                output_tokens = 2000, cached_input_tokens = 8000,
                cost_usd = 0.0345, ended_at = ?1
             WHERE id = 'cost-sess'",
            params![now.to_rfc3339()],
        )?;

        let mut stmt = conn.prepare(
            "SELECT id, agent_definition_id, agent_definition_name, session_key,
                    parent_session_id, thread_id, source_channel, status, model,
                    turn_count, input_tokens, output_tokens, cached_input_tokens,
                    cost_usd, transcript_path, started_at, ended_at
             FROM sessions WHERE id = 'cost-sess'",
        )?;
        let session = stmt.query_row([], map_session_row)?;

        assert_eq!(session.status, SessionStatus::Completed);
        assert_eq!(session.turn_count, 5);
        assert_eq!(session.input_tokens, 10000);
        assert_eq!(session.output_tokens, 2000);
        assert_eq!(session.cached_input_tokens, 8000);
        assert!((session.cost_usd - 0.0345).abs() < f64::EPSILON);
        assert!(session.ended_at.is_some());

        Ok(())
    })
    .unwrap();
}

#[test]
fn combined_filters() {
    with_memory_connection(|conn| {
        insert_test_session(conn, "cf1", "orchestrator", "key1");
        insert_test_session(conn, "cf2", "orchestrator", "key2");
        insert_test_session(conn, "cf3", "researcher", "key3");

        conn.execute(
            "UPDATE sessions SET status = 'completed' WHERE id = 'cf1'",
            [],
        )?;

        let params = SessionSearchParams {
            agent_id: Some("orchestrator".to_string()),
            status: Some("completed".to_string()),
            ..Default::default()
        };

        let result = search_sessions_inner(conn, &params)?;
        assert_eq!(result.total, 1);
        assert_eq!(result.sessions[0].id, "cf1");
        Ok(())
    })
    .unwrap();
}
