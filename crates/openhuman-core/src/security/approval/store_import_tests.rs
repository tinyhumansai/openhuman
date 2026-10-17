use super::*;
use crate::security::approval::store as live;
use crate::security::approval::types::{ApprovalDecision, ApprovalSourceContext};
use crate::storage::import::{run_with, Outcome};
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

fn import(config: &Config) -> Outcome {
    let backend = crate::storage::local::backend_for(&config.workspace_dir).unwrap();
    run_with(&backend, config, &[super::super::import_source()])
        .remove(0)
        .outcome
        .unwrap()
}

#[test]
fn legacy_rows_are_imported_once_and_the_file_is_left_alone() {
    let (_dir, config) = workspace();
    seed_request(&config, "pending-1", 10, None, false);
    seed_request(&config, "approved-1", 10, Some("approve_once"), true);
    seed_request(&config, "denied-1", 10, Some("deny"), false);
    seed_request(&config, "stale-1", -5, None, false);
    seed_trust(&config, "flow-1", "shell");
    seed_trust(&config, "flow-1", "http");
    let db = db_path(&config);
    let before = std::fs::read(&db).unwrap();

    assert!(matches!(
        import(&config),
        Outcome::Imported {
            found: 6,
            written: 6
        }
    ));

    let pending = live::list_pending(&config).unwrap();
    let mut ids: Vec<_> = pending.iter().map(|p| p.request_id.as_str()).collect();
    ids.sort_unstable();
    // Listing expires the stale request, as it always did.
    assert_eq!(ids, ["pending-1"]);
    let first = &pending[0];
    assert_eq!(first.tool_name, "composio");
    assert_eq!(first.action_summary, "summary pending-1");
    assert_eq!(first.args_redacted["id"], "pending-1");
    assert_eq!(first.tool_call_id.as_deref(), Some("call-pending-1"));
    assert_eq!(first.agent_id.as_deref(), Some("agent-7"));
    assert!(matches!(
        first.source_context,
        Some(ApprovalSourceContext::Flow { ref flow_id, .. }) if flow_id == "flow-1"
    ));
    assert_eq!(
        live::get_decision(&config, "approved-1").unwrap(),
        Some(ApprovalDecision::ApproveOnce)
    );
    assert_eq!(
        live::get_decision(&config, "denied-1").unwrap(),
        Some(ApprovalDecision::Deny)
    );
    assert!(live::is_flow_tool_trusted(&config, "flow-1", "shell").unwrap());
    assert!(live::is_flow_tool_trusted(&config, "flow-1", "http").unwrap());
    assert!(!live::is_flow_tool_trusted(&config, "flow-2", "http").unwrap());
    // The expiry key was imported: the stale request was expired.
    assert_eq!(
        live::get_decision(&config, "stale-1").unwrap(),
        Some(ApprovalDecision::Deny)
    );
    assert_eq!(std::fs::read(&db).unwrap(), before, "approval.db untouched");

    // A second run does not import again: a change made since stays made.
    live::decide(&config, "pending-1", ApprovalDecision::Deny).unwrap();
    assert!(matches!(import(&config), Outcome::AlreadyDone));
    assert_eq!(
        live::get_decision(&config, "pending-1").unwrap(),
        Some(ApprovalDecision::Deny)
    );
    assert!(live::list_pending(&config).unwrap().is_empty());
}

#[test]
fn a_workspace_with_no_legacy_file_starts_empty() {
    let (_dir, config) = workspace();
    assert!(matches!(import(&config), Outcome::NoSource));
    assert!(live::list_pending(&config).unwrap().is_empty());
}

#[test]
fn an_old_schema_is_brought_forward_and_its_session_ids_scrubbed() {
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

    assert!(matches!(import(&config), Outcome::Imported { .. }));
    let pending = live::list_pending(&config).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].request_id, "old");
    assert!(
        pending[0].agent_id.is_none(),
        "an old row is the process's own"
    );
    // The credential-shaped session id was scrubbed before it was read.
    let docs = crate::storage::local::open("test", &config.workspace_dir)
        .unwrap()
        .scoped()
        .unwrap();
    let docs = std::sync::Arc::clone(docs.documents());
    let stored =
        crate::storage::block_on(async move { docs.get("approvals.requests", "old").await })
            .unwrap()
            .unwrap();
    assert_eq!(stored.doc["session_id"], PRE_MIGRATION_SESSION_ID);
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
