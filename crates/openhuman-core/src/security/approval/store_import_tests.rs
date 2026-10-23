use super::*;
use crate::security::approval::types::ApprovalSourceContext;
use crate::storage::rusqlite;
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

/// Writes a `pending_approvals` row the way the retired store did.
#[allow(clippy::too_many_arguments)]
fn seed_request(
    config: &Config,
    id: &str,
    expires_in_minutes: i64,
    decision: Option<&str>,
    executed: bool,
) {
    with_connection(config, |conn| {
        let now = Utc::now();
        conn.execute(
            "INSERT INTO pending_approvals
                (request_id, tool_name, action_summary, args_redacted, session_id,
                 created_at, expires_at, decided_at, decision, executed_at,
                 execution_outcome, source_context, tool_call_id, agent_id)
             VALUES (?1, 'composio', ?2, ?3, 'sess', ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'agent-7')",
            params![
                id,
                format!("summary {id}"),
                format!(r#"{{"action":"execute","id":"{id}"}}"#),
                now.to_rfc3339(),
                (now + Duration::minutes(expires_in_minutes)).to_rfc3339(),
                decision.map(|_| now.to_rfc3339()),
                decision,
                executed.then(|| now.to_rfc3339()),
                executed.then_some("success"),
                serde_json::to_string(&ApprovalSourceContext::Flow {
                    flow_id: "flow-1".into(),
                    run_id: "run-1".into(),
                    node_id: None,
                })
                .unwrap(),
                format!("call-{id}"),
            ],
        )?;
        Ok(())
    })
    .unwrap();
}

fn seed_trust(config: &Config, flow: &str, tool: &str) {
    with_connection(config, |conn| {
        conn.execute(
            "INSERT OR IGNORE INTO flow_tool_trust (flow_id, tool_name, created_at)
             VALUES (?1, ?2, ?3)",
            params![flow, tool, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    })
    .unwrap();
}

#[test]
fn legacy_rows_are_read_in_order_from_a_scratch_copy_and_the_file_is_left_alone() {
    let (_dir, config) = workspace();
    seed_request(&config, "pending-1", 10, None, false);
    seed_request(&config, "approved-1", 10, Some("approve_once"), true);
    seed_request(&config, "denied-1", 10, Some("deny"), false);
    seed_request(&config, "stale-1", -5, None, false);
    seed_trust(&config, "flow-1", "shell");
    seed_trust(&config, "flow-1", "http");
    let db = db_path(&config);
    let before = std::fs::read(&db).unwrap();

    let history = read_legacy(&config).unwrap().expect("a legacy file");

    let ids: Vec<&str> = history
        .approvals
        .iter()
        .map(|row| row.request_id.as_str())
        .collect();
    assert_eq!(ids, ["pending-1", "approved-1", "denied-1", "stale-1"]);
    let approved = &history.approvals[1];
    assert_eq!(approved.tool_name, "composio");
    assert_eq!(approved.agent_id.as_deref(), Some("agent-7"));
    assert_eq!(approved.decision.as_deref(), Some("approve_once"));
    assert!(approved.executed && approved.decided_at.is_some());
    let pending = &history.approvals[0];
    assert!(pending.decision.is_none() && !pending.executed && pending.expires_at.is_some());
    assert!(history.approvals[3].expires_at.unwrap() < Utc::now());
    assert_eq!(
        history.trust,
        [
            LegacyTrust {
                flow_id: "flow-1".into(),
                tool_name: "shell".into()
            },
            LegacyTrust {
                flow_id: "flow-1".into(),
                tool_name: "http".into()
            }
        ]
    );
    assert_eq!(std::fs::read(&db).unwrap(), before, "approval.db untouched");
}

#[test]
fn a_workspace_with_no_legacy_file_reads_as_none_and_creates_nothing() {
    let (_dir, config) = workspace();
    assert!(read_legacy(&config).unwrap().is_none());
    assert!(!db_path(&config).exists());
}

#[test]
fn an_old_schema_is_brought_forward_on_the_copy_only() {
    let (_dir, config) = workspace();
    let db = db_path(&config);
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    {
        // The first schema: no audit/agent columns, and no flow_tool_trust.
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE pending_approvals (
                request_id TEXT PRIMARY KEY, tool_name TEXT NOT NULL,
                action_summary TEXT NOT NULL, args_redacted TEXT NOT NULL,
                session_id TEXT NOT NULL, created_at TEXT NOT NULL,
                expires_at TEXT, decided_at TEXT, decision TEXT);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO pending_approvals
                (request_id, tool_name, action_summary, args_redacted, session_id, created_at)
             VALUES ('old', 'composio', 'old row', '{}', 'a-credential-shaped-value', ?1)",
            [Utc::now().to_rfc3339()],
        )
        .unwrap();
    }
    let before = std::fs::read(&db).unwrap();

    let history = read_legacy(&config).unwrap().expect("a legacy file");
    assert_eq!(history.approvals.len(), 1);
    assert!(
        history.approvals[0].agent_id.is_none(),
        "an old row is the process's own"
    );
    assert!(history.trust.is_empty());
    assert_eq!(std::fs::read(&db).unwrap(), before, "approval.db untouched");
}

#[test]
fn the_schema_migration_surfaces_non_duplicate_alter_errors() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE VIEW pending_approvals AS SELECT 1 AS id")
        .unwrap();
    let error = migrate_columns(&conn).expect_err("a view cannot be migrated as a table");
    assert!(error.to_string().contains("add column executed_at"));
}
