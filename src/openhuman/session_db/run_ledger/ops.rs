use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use crate::openhuman::config::Config;

use super::store::init_run_ledger_schema;
use super::types::{
    AgentRun, AgentRunListRequest, AgentRunListResponse, AgentRunStatus, AgentRunUpsert, RunEvent,
    RunEventAppend, RunEventListRequest, RunEventListResponse, RunTelemetry, RunTelemetryUpsert,
    WorkflowRun, WorkflowRunUpsert,
};

const LOG_PREFIX: &str = "[session_db:run_ledger]";

pub fn upsert_agent_run(config: &Config, upsert: AgentRunUpsert) -> Result<AgentRun> {
    let now = Utc::now();
    let started_at = upsert.started_at.unwrap_or(now);
    let updated_at = now;
    let metadata_json =
        serde_json::to_string(&upsert.metadata).context("serialize agent run metadata")?;
    let checkpoint_json = upsert
        .checkpoint
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .context("serialize agent run checkpoint")?;

    log::debug!(
        "{LOG_PREFIX} upsert_agent_run id={} kind={} status={} parent={} thread={}",
        upsert.id,
        upsert.kind.as_str(),
        upsert.status.as_str(),
        upsert.parent_run_id.as_deref().unwrap_or("-"),
        upsert.parent_thread_id.as_deref().unwrap_or("-")
    );

    crate::openhuman::session_db::store::with_connection(config, |conn| {
        init_run_ledger_schema(conn)?;
        conn.execute(
            "INSERT INTO agent_runs (
                id, kind, parent_run_id, parent_thread_id, agent_id, status,
                prompt_ref, worker_thread_id, task_board_id, task_card_id,
                checkpoint_path, checkpoint_json, summary, error, metadata_json,
                started_at, updated_at, completed_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
             ON CONFLICT(id) DO UPDATE SET
                kind = CASE
                    WHEN agent_runs.kind = 'worker_thread' AND excluded.kind = 'subagent' THEN agent_runs.kind
                    ELSE excluded.kind
                END,
                parent_run_id = COALESCE(excluded.parent_run_id, agent_runs.parent_run_id),
                parent_thread_id = COALESCE(excluded.parent_thread_id, agent_runs.parent_thread_id),
                agent_id = COALESCE(excluded.agent_id, agent_runs.agent_id),
                status = excluded.status,
                prompt_ref = COALESCE(excluded.prompt_ref, agent_runs.prompt_ref),
                worker_thread_id = COALESCE(excluded.worker_thread_id, agent_runs.worker_thread_id),
                task_board_id = COALESCE(excluded.task_board_id, agent_runs.task_board_id),
                task_card_id = COALESCE(excluded.task_card_id, agent_runs.task_card_id),
                checkpoint_path = COALESCE(excluded.checkpoint_path, agent_runs.checkpoint_path),
                checkpoint_json = COALESCE(excluded.checkpoint_json, agent_runs.checkpoint_json),
                summary = COALESCE(excluded.summary, agent_runs.summary),
                error = COALESCE(excluded.error, agent_runs.error),
                metadata_json = CASE
                    WHEN excluded.metadata_json = '{}' THEN agent_runs.metadata_json
                    ELSE excluded.metadata_json
                END,
                updated_at = excluded.updated_at,
                completed_at = COALESCE(excluded.completed_at, agent_runs.completed_at)",
            params![
                upsert.id,
                upsert.kind.as_str(),
                upsert.parent_run_id,
                upsert.parent_thread_id,
                upsert.agent_id,
                upsert.status.as_str(),
                upsert.prompt_ref,
                upsert.worker_thread_id,
                upsert.task_board_id,
                upsert.task_card_id,
                upsert.checkpoint_path,
                checkpoint_json,
                upsert.summary,
                upsert.error,
                metadata_json,
                started_at.to_rfc3339(),
                updated_at.to_rfc3339(),
                upsert.completed_at.map(|dt| dt.to_rfc3339()),
            ],
        )
        .context("upsert agent run")?;
        Ok(())
    })?;

    get_agent_run(config, &upsert.id)?.context("agent run missing after upsert")
}

pub fn upsert_workflow_run(config: &Config, upsert: WorkflowRunUpsert) -> Result<WorkflowRun> {
    let now = Utc::now();
    let started_at = upsert.started_at.unwrap_or(now);
    let input_json = serde_json::to_string(&upsert.input).context("serialize workflow input")?;
    let phase_states_json =
        serde_json::to_string(&upsert.phase_states).context("serialize workflow phase states")?;
    let child_run_ids_json =
        serde_json::to_string(&upsert.child_run_ids).context("serialize child run ids")?;

    crate::openhuman::session_db::store::with_connection(config, |conn| {
        init_run_ledger_schema(conn)?;
        conn.execute(
            "INSERT INTO workflow_runs (
                id, definition_id, parent_thread_id, input_json, phase_states_json,
                child_run_ids_json, status, summary, started_at, updated_at, completed_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
                definition_id = excluded.definition_id,
                parent_thread_id = COALESCE(excluded.parent_thread_id, workflow_runs.parent_thread_id),
                input_json = excluded.input_json,
                phase_states_json = excluded.phase_states_json,
                child_run_ids_json = excluded.child_run_ids_json,
                status = excluded.status,
                summary = COALESCE(excluded.summary, workflow_runs.summary),
                updated_at = excluded.updated_at,
                completed_at = COALESCE(excluded.completed_at, workflow_runs.completed_at)",
            params![
                upsert.id,
                upsert.definition_id,
                upsert.parent_thread_id,
                input_json,
                phase_states_json,
                child_run_ids_json,
                upsert.status.as_str(),
                upsert.summary,
                started_at.to_rfc3339(),
                now.to_rfc3339(),
                upsert.completed_at.map(|dt| dt.to_rfc3339()),
            ],
        )
        .context("upsert workflow run")?;
        Ok(())
    })?;

    get_workflow_run(config, &upsert.id)?.context("workflow run missing after upsert")
}

pub fn append_run_event(config: &Config, event: RunEventAppend) -> Result<RunEvent> {
    let now = Utc::now();
    let payload_json = serde_json::to_string(&event.payload).context("serialize run event")?;
    crate::openhuman::session_db::store::with_connection(config, |conn| {
        init_run_ledger_schema(conn)?;
        let next_sequence: i64 = conn.query_row(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM run_events WHERE run_id = ?1",
            params![event.run_id],
            |row| row.get(0),
        )?;
        conn.execute(
            "INSERT INTO run_events (run_id, sequence, event_type, payload_json, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                event.run_id,
                next_sequence,
                event.event_type,
                payload_json,
                now.to_rfc3339(),
            ],
        )
        .context("append run event")?;
        Ok(RunEvent {
            run_id: event.run_id,
            sequence: next_sequence as u64,
            event_type: event.event_type,
            payload: serde_json::from_str(&payload_json).unwrap_or_else(|_| json!({})),
            timestamp: now,
        })
    })
}

pub fn upsert_run_telemetry(config: &Config, upsert: RunTelemetryUpsert) -> Result<RunTelemetry> {
    let now = Utc::now();
    crate::openhuman::session_db::store::with_connection(config, |conn| {
        init_run_ledger_schema(conn)?;
        conn.execute(
            "INSERT INTO run_telemetry (
                run_id, input_tokens, output_tokens, cached_input_tokens, cost_usd,
                elapsed_ms, tool_count, model, provider, error, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(run_id) DO UPDATE SET
                input_tokens = COALESCE(excluded.input_tokens, run_telemetry.input_tokens),
                output_tokens = COALESCE(excluded.output_tokens, run_telemetry.output_tokens),
                cached_input_tokens = COALESCE(excluded.cached_input_tokens, run_telemetry.cached_input_tokens),
                cost_usd = COALESCE(excluded.cost_usd, run_telemetry.cost_usd),
                elapsed_ms = COALESCE(excluded.elapsed_ms, run_telemetry.elapsed_ms),
                tool_count = COALESCE(excluded.tool_count, run_telemetry.tool_count),
                model = COALESCE(excluded.model, run_telemetry.model),
                provider = COALESCE(excluded.provider, run_telemetry.provider),
                error = COALESCE(excluded.error, run_telemetry.error),
                updated_at = excluded.updated_at",
            params![
                upsert.run_id,
                upsert.input_tokens.map(|v| v as i64),
                upsert.output_tokens.map(|v| v as i64),
                upsert.cached_input_tokens.map(|v| v as i64),
                upsert.cost_usd,
                upsert.elapsed_ms.map(|v| v as i64),
                upsert.tool_count.map(|v| v as i64),
                upsert.model,
                upsert.provider,
                upsert.error,
                now.to_rfc3339(),
            ],
        )
        .context("upsert run telemetry")?;
        get_run_telemetry_inner(conn, &upsert.run_id)
    })
}

pub fn get_agent_run(config: &Config, id: &str) -> Result<Option<AgentRun>> {
    crate::openhuman::session_db::store::with_connection(config, |conn| {
        init_run_ledger_schema(conn)?;
        get_agent_run_inner(conn, id)
    })
}

pub fn list_agent_runs(
    config: &Config,
    request: &AgentRunListRequest,
) -> Result<AgentRunListResponse> {
    crate::openhuman::session_db::store::with_connection(config, |conn| {
        init_run_ledger_schema(conn)?;
        let mut where_clauses = Vec::new();
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(status) = request.status.as_deref().filter(|s| !s.trim().is_empty()) {
            values.push(Box::new(status.to_string()));
            where_clauses.push(format!("status = ?{}", values.len()));
        }
        if let Some(kind) = request.kind.as_deref().filter(|s| !s.trim().is_empty()) {
            values.push(Box::new(kind.to_string()));
            where_clauses.push(format!("kind = ?{}", values.len()));
        }
        if let Some(parent) = request
            .parent_run_id
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            values.push(Box::new(parent.to_string()));
            where_clauses.push(format!("parent_run_id = ?{}", values.len()));
        }
        if let Some(thread) = request
            .parent_thread_id
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            values.push(Box::new(thread.to_string()));
            where_clauses.push(format!("parent_thread_id = ?{}", values.len()));
        }

        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_clauses.join(" AND "))
        };
        let count_sql = format!("SELECT COUNT(*) FROM agent_runs {where_sql}");
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            values.iter().map(|v| v.as_ref()).collect();
        let count = conn.query_row(&count_sql, params_ref.as_slice(), |row| {
            row.get::<_, i64>(0)
        })? as usize;

        let limit = request.limit.unwrap_or(50).min(500) as i64;
        let offset = request.offset.unwrap_or(0) as i64;
        values.push(Box::new(limit));
        let limit_idx = values.len();
        values.push(Box::new(offset));
        let offset_idx = values.len();

        let query_sql = format!(
            "SELECT id, kind, parent_run_id, parent_thread_id, agent_id, status,
                    prompt_ref, worker_thread_id, task_board_id, task_card_id,
                    checkpoint_path, checkpoint_json, summary, error, metadata_json,
                    started_at, updated_at, completed_at
             FROM agent_runs {where_sql}
             ORDER BY updated_at DESC
             LIMIT ?{limit_idx} OFFSET ?{offset_idx}"
        );
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            values.iter().map(|v| v.as_ref()).collect();
        let mut stmt = conn.prepare(&query_sql)?;
        let rows = stmt.query_map(params_ref.as_slice(), |row| map_agent_run_row(conn, row))?;
        let mut runs = Vec::new();
        for row in rows {
            runs.push(row?);
        }
        Ok(AgentRunListResponse { runs, count })
    })
}

pub fn list_recent_run_events(
    config: &Config,
    request: &RunEventListRequest,
) -> Result<RunEventListResponse> {
    crate::openhuman::session_db::store::with_connection(config, |conn| {
        init_run_ledger_schema(conn)?;
        let limit = request.limit.unwrap_or(100).min(1000) as i64;
        let after = request.after_sequence.unwrap_or(0) as i64;
        let mut stmt = conn.prepare(
            "SELECT run_id, sequence, event_type, payload_json, timestamp
             FROM run_events
             WHERE run_id = ?1 AND sequence > ?2
             ORDER BY sequence ASC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![request.run_id, after, limit], map_run_event_row)?;
        let mut events = Vec::new();
        for row in rows {
            events.push(row?);
        }
        Ok(RunEventListResponse {
            count: events.len(),
            events,
        })
    })
}

fn get_workflow_run(config: &Config, id: &str) -> Result<Option<WorkflowRun>> {
    crate::openhuman::session_db::store::with_connection(config, |conn| {
        init_run_ledger_schema(conn)?;
        let mut stmt = conn.prepare(
            "SELECT id, definition_id, parent_thread_id, input_json, phase_states_json,
                    child_run_ids_json, status, summary, started_at, updated_at, completed_at
             FROM workflow_runs WHERE id = ?1",
        )?;
        Ok(stmt
            .query_row(params![id], map_workflow_run_row)
            .optional()?)
    })
}

fn get_agent_run_inner(conn: &Connection, id: &str) -> Result<Option<AgentRun>> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, parent_run_id, parent_thread_id, agent_id, status,
                prompt_ref, worker_thread_id, task_board_id, task_card_id,
                checkpoint_path, checkpoint_json, summary, error, metadata_json,
                started_at, updated_at, completed_at
         FROM agent_runs WHERE id = ?1",
    )?;
    stmt.query_row(params![id], |row| map_agent_run_row(conn, row))
        .optional()
        .map_err(Into::into)
}

fn get_run_telemetry_inner(conn: &Connection, run_id: &str) -> Result<RunTelemetry> {
    let mut stmt = conn.prepare(
        "SELECT run_id, input_tokens, output_tokens, cached_input_tokens, cost_usd,
                elapsed_ms, tool_count, model, provider, error, updated_at
         FROM run_telemetry WHERE run_id = ?1",
    )?;
    stmt.query_row(params![run_id], map_run_telemetry_row)
        .context("run telemetry missing after upsert")
}

fn get_optional_run_telemetry(
    conn: &Connection,
    run_id: &str,
) -> rusqlite::Result<Option<RunTelemetry>> {
    let mut stmt = conn.prepare(
        "SELECT run_id, input_tokens, output_tokens, cached_input_tokens, cost_usd,
                elapsed_ms, tool_count, model, provider, error, updated_at
         FROM run_telemetry WHERE run_id = ?1",
    )?;
    stmt.query_row(params![run_id], map_run_telemetry_row)
        .optional()
}

fn map_agent_run_row(conn: &Connection, row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentRun> {
    let id: String = row.get(0)?;
    let checkpoint_json: Option<String> = row.get(11)?;
    let metadata_json: String = row.get(14)?;
    Ok(AgentRun {
        id: id.clone(),
        kind: super::types::AgentRunKind::parse(&row.get::<_, String>(1)?),
        parent_run_id: row.get(2)?,
        parent_thread_id: row.get(3)?,
        agent_id: row.get(4)?,
        status: AgentRunStatus::parse(&row.get::<_, String>(5)?),
        prompt_ref: row.get(6)?,
        worker_thread_id: row.get(7)?,
        task_board_id: row.get(8)?,
        task_card_id: row.get(9)?,
        checkpoint_path: row.get(10)?,
        checkpoint: parse_json_opt(checkpoint_json),
        summary: row.get(12)?,
        error: row.get(13)?,
        metadata: parse_json(metadata_json),
        telemetry: get_optional_run_telemetry(conn, &id)?,
        started_at: parse_rfc3339(&row.get::<_, String>(15)?)?,
        updated_at: parse_rfc3339(&row.get::<_, String>(16)?)?,
        completed_at: parse_rfc3339_opt(row.get(17)?)?,
    })
}

fn map_workflow_run_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowRun> {
    Ok(WorkflowRun {
        id: row.get(0)?,
        definition_id: row.get(1)?,
        parent_thread_id: row.get(2)?,
        input: parse_json(row.get(3)?),
        phase_states: parse_json(row.get(4)?),
        child_run_ids: serde_json::from_str(&row.get::<_, String>(5)?).unwrap_or_default(),
        status: super::types::WorkflowRunStatus::parse(&row.get::<_, String>(6)?),
        summary: row.get(7)?,
        started_at: parse_rfc3339(&row.get::<_, String>(8)?)?,
        updated_at: parse_rfc3339(&row.get::<_, String>(9)?)?,
        completed_at: parse_rfc3339_opt(row.get(10)?)?,
    })
}

fn map_run_event_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunEvent> {
    Ok(RunEvent {
        run_id: row.get(0)?,
        sequence: row.get::<_, i64>(1)? as u64,
        event_type: row.get(2)?,
        payload: parse_json(row.get(3)?),
        timestamp: parse_rfc3339(&row.get::<_, String>(4)?)?,
    })
}

fn map_run_telemetry_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunTelemetry> {
    Ok(RunTelemetry {
        run_id: row.get(0)?,
        input_tokens: row.get::<_, i64>(1)? as u64,
        output_tokens: row.get::<_, i64>(2)? as u64,
        cached_input_tokens: row.get::<_, i64>(3)? as u64,
        cost_usd: row.get(4)?,
        elapsed_ms: row.get::<_, Option<i64>>(5)?.map(|v| v as u64),
        tool_count: row.get::<_, i64>(6)? as u64,
        model: row.get(7)?,
        provider: row.get(8)?,
        error: row.get(9)?,
        updated_at: Some(parse_rfc3339(&row.get::<_, String>(10)?)?),
    })
}

fn parse_json(raw: String) -> Value {
    serde_json::from_str(&raw).unwrap_or_else(|_| json!({}))
}

fn parse_json_opt(raw: Option<String>) -> Option<Value> {
    raw.and_then(|value| serde_json::from_str(&value).ok())
}

fn parse_rfc3339(raw: &str) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(err))
        })
}

fn parse_rfc3339_opt(raw: Option<String>) -> rusqlite::Result<Option<DateTime<Utc>>> {
    match raw {
        Some(value) => parse_rfc3339(&value).map(Some),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config(dir: &TempDir) -> Config {
        let mut config = Config::default();
        config.workspace_dir = dir.path().to_path_buf();
        config.action_dir = dir.path().join("actions");
        config
    }

    #[test]
    fn agent_run_append_list_get_and_events_are_ordered() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);

        let run = upsert_agent_run(
            &config,
            AgentRunUpsert {
                id: "run-1".into(),
                kind: super::super::types::AgentRunKind::Subagent,
                parent_run_id: Some("parent".into()),
                parent_thread_id: Some("thread-1".into()),
                agent_id: Some("researcher".into()),
                status: AgentRunStatus::Running,
                prompt_ref: Some("worker-1:user:seed".into()),
                worker_thread_id: Some("worker-1".into()),
                task_board_id: None,
                task_card_id: None,
                checkpoint_path: None,
                checkpoint: None,
                summary: None,
                error: None,
                metadata: json!({"source": "test"}),
                started_at: None,
                completed_at: None,
            },
        )
        .unwrap();
        assert_eq!(run.status, AgentRunStatus::Running);

        append_run_event(
            &config,
            RunEventAppend {
                run_id: "run-1".into(),
                event_type: "spawned".into(),
                payload: json!({"agentId": "researcher"}),
            },
        )
        .unwrap();
        append_run_event(
            &config,
            RunEventAppend {
                run_id: "run-1".into(),
                event_type: "completed".into(),
                payload: json!({"elapsedMs": 12}),
            },
        )
        .unwrap();

        let events = list_recent_run_events(
            &config,
            &RunEventListRequest {
                run_id: "run-1".into(),
                after_sequence: Some(0),
                limit: None,
            },
        )
        .unwrap();
        assert_eq!(events.events.len(), 2);
        assert_eq!(events.events[0].sequence, 1);
        assert_eq!(events.events[1].sequence, 2);

        let list = list_agent_runs(
            &config,
            &AgentRunListRequest {
                parent_thread_id: Some("thread-1".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(list.count, 1);
        assert_eq!(list.runs[0].worker_thread_id.as_deref(), Some("worker-1"));
    }
}
