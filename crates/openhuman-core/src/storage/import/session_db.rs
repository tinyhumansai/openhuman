//! Read-only decoder for the historical `session_db/sessions.db` schema.
//!
//! The legacy database belongs to the host migration boundary. This module
//! reads a consistent scratch snapshot and returns the current TinyAgents
//! record types; it does not open or mutate owner storage.

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::de::DeserializeOwned;
use serde_json::{json, Map, Number, Value};
use tinyagents_session::run_ledger::tool_effects::{ToolEffectRow, ToolEffectStatus};
use tinyagents_session::run_ledger::{
    AgentRun, AgentTeam, AgentTeamMember, AgentTeamTask, RunEvent, RunTelemetry, WorkflowRun,
};
use tinyagents_session::{SessionMessage, SessionRecord, SessionToolCall};

use crate::storage::rusqlite::{self, types::ValueRef};

/// Typed rows from the historical session database, in deterministic source
/// order. The host passes these rows to TinyAgents' typed, idempotent import
/// ports after reading the consistent snapshot.
#[derive(Debug, Default)]
pub(crate) struct SessionDatabase {
    pub sessions: Vec<SessionRecord>,
    pub messages: Vec<SessionMessage>,
    pub tool_calls: Vec<SessionToolCall>,
    pub agent_runs: Vec<AgentRun>,
    pub telemetry: Vec<RunTelemetry>,
    pub events: Vec<RunEvent>,
    pub workflows: Vec<WorkflowRun>,
    pub teams: Vec<AgentTeam>,
    pub members: Vec<AgentTeamMember>,
    pub tasks: Vec<AgentTeamTask>,
    pub tool_effects: Vec<ToolEffectRow>,
}

#[derive(Clone, Copy)]
enum Decode {
    Plain,
    JsonOrEmpty,
    OptionalJson,
    JsonArray,
}

#[derive(Clone, Copy)]
struct Field {
    column: &'static str,
    output: &'static str,
    fallback: &'static str,
    decode: Decode,
}

const fn field(column: &'static str, fallback: &'static str) -> Field {
    Field {
        column,
        output: column,
        fallback,
        decode: Decode::Plain,
    }
}

const fn renamed(
    column: &'static str,
    output: &'static str,
    fallback: &'static str,
    decode: Decode,
) -> Field {
    Field {
        column,
        output,
        fallback,
        decode,
    }
}

impl SessionDatabase {
    /// Reads all supported historical tables from a consistent scratch
    /// snapshot. Missing tables are empty; a database with none of the
    /// recognized tables is rejected.
    pub(crate) fn read(path: &Path) -> Result<Self> {
        let connection = rusqlite::Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| {
            format!(
                "open session database snapshot {} read-only",
                path.display()
            )
        })?;
        let known = [
            "sessions",
            "session_messages",
            "agent_runs",
            "workflow_runs",
            "agent_teams",
            "tool_effects",
        ];
        if !known
            .iter()
            .map(|table| table_exists(&connection, table))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .any(|exists| exists)
        {
            bail!("source has none of the classic session database tables");
        }

        Ok(Self {
            sessions: read_records(
                &connection,
                "sessions",
                "started_at, id",
                &[
                    field("id", "NULL"),
                    field("agent_definition_id", "''"),
                    field("agent_definition_name", "''"),
                    field("session_key", "''"),
                    field("parent_session_id", "NULL"),
                    field("thread_id", "NULL"),
                    field("source_channel", "NULL"),
                    field("status", "'running'"),
                    field("model", "NULL"),
                    field("turn_count", "0"),
                    field("input_tokens", "0"),
                    field("output_tokens", "0"),
                    field("cached_input_tokens", "0"),
                    field("cost_usd", "0.0"),
                    field("transcript_path", "NULL"),
                    field("started_at", "NULL"),
                    field("ended_at", "NULL"),
                ],
            )?,
            messages: read_records(
                &connection,
                "session_messages",
                "id",
                &[
                    field("id", "NULL"),
                    field("session_id", "NULL"),
                    field("role", "''"),
                    field("content", "''"),
                    field("reasoning_content", "NULL"),
                    field("model", "NULL"),
                    field("input_tokens", "NULL"),
                    field("output_tokens", "NULL"),
                    field("cost_usd", "NULL"),
                    field("created_at", "NULL"),
                ],
            )?,
            tool_calls: read_records(
                &connection,
                "session_tool_calls",
                "id",
                &[
                    field("id", "NULL"),
                    field("session_id", "NULL"),
                    field("message_id", "NULL"),
                    field("tool_name", "''"),
                    field("tool_input", "NULL"),
                    field("tool_output", "NULL"),
                    field("status", "'pending'"),
                    field("duration_ms", "NULL"),
                    field("created_at", "NULL"),
                ],
            )?,
            agent_runs: read_records(
                &connection,
                "agent_runs",
                "id",
                &[
                    field("id", "NULL"),
                    field("kind", "''"),
                    field("parent_run_id", "NULL"),
                    field("parent_thread_id", "NULL"),
                    field("agent_id", "NULL"),
                    field("status", "''"),
                    field("prompt_ref", "NULL"),
                    field("worker_thread_id", "NULL"),
                    field("checkpoint_path", "NULL"),
                    renamed(
                        "checkpoint_json",
                        "checkpoint",
                        "NULL",
                        Decode::OptionalJson,
                    ),
                    field("summary", "NULL"),
                    field("error", "NULL"),
                    renamed("metadata_json", "metadata", "'{}'", Decode::JsonOrEmpty),
                    field("started_at", "NULL"),
                    field("updated_at", "NULL"),
                    field("completed_at", "NULL"),
                ],
            )?,
            telemetry: read_records(
                &connection,
                "run_telemetry",
                "run_id",
                &[
                    field("run_id", "NULL"),
                    field("input_tokens", "0"),
                    field("output_tokens", "0"),
                    field("cached_input_tokens", "0"),
                    field("cost_usd", "0.0"),
                    field("elapsed_ms", "NULL"),
                    field("tool_count", "0"),
                    field("model", "NULL"),
                    field("provider", "NULL"),
                    field("error", "NULL"),
                    field("updated_at", "NULL"),
                ],
            )?,
            events: read_records(
                &connection,
                "run_events",
                "run_id, sequence",
                &[
                    field("run_id", "NULL"),
                    field("sequence", "NULL"),
                    field("event_type", "''"),
                    renamed("payload_json", "payload", "'{}'", Decode::JsonOrEmpty),
                    renamed("timestamp", "timestamp", "NULL", Decode::Plain),
                ],
            )?,
            workflows: read_records(
                &connection,
                "workflow_runs",
                "id",
                &[
                    field("id", "NULL"),
                    field("definition_id", "''"),
                    field("parent_thread_id", "NULL"),
                    renamed("input_json", "input", "'{}'", Decode::JsonOrEmpty),
                    renamed(
                        "phase_states_json",
                        "phase_states",
                        "'{}'",
                        Decode::JsonOrEmpty,
                    ),
                    renamed(
                        "child_run_ids_json",
                        "child_run_ids",
                        "'[]'",
                        Decode::JsonArray,
                    ),
                    field("status", "''"),
                    field("summary", "NULL"),
                    field("started_at", "NULL"),
                    field("updated_at", "NULL"),
                    field("completed_at", "NULL"),
                    field("revision", "0"),
                    field("lease_owner", "NULL"),
                    field("lease_expires_at", "NULL"),
                ],
            )?,
            teams: read_records(
                &connection,
                "agent_teams",
                "id",
                &[
                    field("id", "NULL"),
                    field("parent_thread_id", "NULL"),
                    field("lead_agent_id", "''"),
                    field("status", "''"),
                    field("summary", "NULL"),
                    field("created_at", "NULL"),
                    field("updated_at", "NULL"),
                    field("closed_at", "NULL"),
                ],
            )?,
            members: read_records(
                &connection,
                "agent_team_members",
                "id",
                &[
                    field("id", "NULL"),
                    field("team_id", "NULL"),
                    field("name", "''"),
                    field("agent_id", "NULL"),
                    field("member_status", "''"),
                    field("current_task_id", "NULL"),
                    field("worker_thread_id", "NULL"),
                    field("run_id", "NULL"),
                    field("created_at", "NULL"),
                    field("updated_at", "NULL"),
                ],
            )?,
            tasks: read_records(
                &connection,
                "agent_team_tasks",
                "id",
                &[
                    field("id", "NULL"),
                    field("team_id", "NULL"),
                    field("title", "''"),
                    field("objective", "NULL"),
                    field("status", "''"),
                    field("owner_member_id", "NULL"),
                    field("claimed_by_member_id", "NULL"),
                    field("claim_token", "NULL"),
                    renamed("depends_on_json", "depends_on", "'[]'", Decode::JsonArray),
                    field("gate_status", "'pending'"),
                    field("gate_reason", "NULL"),
                    renamed("evidence_json", "evidence", "'[]'", Decode::JsonArray),
                    field("source_run_id", "NULL"),
                    field("order_index", "0"),
                    field("created_at", "NULL"),
                    field("updated_at", "NULL"),
                ],
            )?,
            tool_effects: read_tool_effects(&connection)?,
        })
    }
}

fn read_records<T: DeserializeOwned>(
    connection: &rusqlite::Connection,
    table: &str,
    order_by: &str,
    fields: &[Field],
) -> Result<Vec<T>> {
    if !table_exists(connection, table)? {
        return Ok(Vec::new());
    }
    let columns = table_columns(connection, table)?;
    let select = fields
        .iter()
        .map(|field| {
            let expression = if columns.contains(field.column) {
                format!("\"{}\"", field.column)
            } else {
                field.fallback.to_owned()
            };
            format!("{expression} AS \"{}\"", field.output)
        })
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("SELECT {select} FROM \"{table}\" ORDER BY {order_by}");
    let mut statement = connection
        .prepare(&sql)
        .with_context(|| format!("prepare legacy {table} read"))?;
    let mut rows = statement
        .query([])
        .with_context(|| format!("query legacy {table}"))?;
    let mut records = Vec::new();
    while let Some(row) = rows
        .next()
        .with_context(|| format!("read legacy {table}"))?
    {
        let mut object = Map::new();
        for (index, field) in fields.iter().enumerate() {
            let value = sql_value(row.get_ref(index)?)?;
            let value = decode_value(value, field.decode)
                .with_context(|| format!("decode legacy column `{table}.{}`", field.column))?;
            object.insert(field.output.to_owned(), value);
        }
        normalize_legacy_status(table, &mut object)?;
        let record: T = serde_json::from_value(Value::Object(object))
            .with_context(|| format!("decode legacy {table} row"))?;
        records.push(record);
    }
    Ok(records)
}

fn read_tool_effects(connection: &rusqlite::Connection) -> Result<Vec<ToolEffectRow>> {
    #[derive(serde::Deserialize)]
    struct RawEffect {
        run_id: String,
        call_id: String,
        tool: String,
        status: String,
        idempotency_key: Option<String>,
        effect_summary: Option<String>,
        started_at: chrono::DateTime<chrono::Utc>,
        settled_at: Option<chrono::DateTime<chrono::Utc>>,
    }

    read_records::<RawEffect>(
        connection,
        "tool_effects",
        "run_id, call_id",
        &[
            field("run_id", "NULL"),
            field("call_id", "NULL"),
            field("tool", "''"),
            field("status", "'started'"),
            field("idempotency_key", "NULL"),
            field("effect_summary", "NULL"),
            field("started_at", "NULL"),
            field("settled_at", "NULL"),
        ],
    )?
    .into_iter()
    .map(|row| {
        let status = ToolEffectStatus::parse(&row.status);
        Ok(ToolEffectRow {
            run_id: row.run_id,
            call_id: row.call_id,
            tool: row.tool,
            status,
            idempotency_key: row.idempotency_key,
            effect_summary: row.effect_summary,
            started_at: row.started_at,
            settled_at: row.settled_at,
        })
    })
    .collect()
}

fn decode_value(value: Value, decode: Decode) -> Result<Value> {
    match decode {
        Decode::Plain => Ok(value),
        Decode::JsonOrEmpty => {
            let raw = value.as_str().context("legacy JSON column is not text")?;
            serde_json::from_str(raw).context("parse legacy JSON column")
        }
        Decode::OptionalJson => match value {
            Value::Null => Ok(Value::Null),
            Value::String(raw) => {
                serde_json::from_str(&raw).context("parse optional legacy JSON column")
            }
            _ => bail!("optional legacy JSON column is not text or NULL"),
        },
        Decode::JsonArray => {
            let raw = value
                .as_str()
                .context("legacy JSON array column is not text")?;
            serde_json::from_str::<Vec<String>>(raw)
                .map(|values| json!(values))
                .context("parse legacy JSON array column")
        }
    }
}

fn normalize_legacy_status(table: &str, row: &mut Map<String, Value>) -> Result<()> {
    let Some(Value::String(status)) = row.get_mut("status") else {
        return Ok(());
    };
    let valid = match table {
        "sessions" => &["running", "completed", "failed", "interrupted"][..],
        "agent_runs" => &[
            "pending",
            "running",
            "awaiting_user",
            "paused",
            "completed",
            "failed",
            "cancelled",
            "interrupted",
        ][..],
        "workflow_runs" => &[
            "pending",
            "running",
            "completed",
            "failed",
            "cancelled",
            "interrupted",
        ][..],
        "agent_teams" => &["active", "closed"][..],
        "agent_team_members" => &["pending", "active", "idle", "stopped"][..],
        "agent_team_tasks" => &["todo", "ready", "in_progress", "blocked", "done"][..],
        _ => return Ok(()),
    };
    if valid.contains(&status.as_str()) {
        return Ok(());
    }
    match table {
        "sessions" => *status = "running".to_owned(),
        "workflow_runs" => *status = "pending".to_owned(),
        "agent_teams" => *status = "active".to_owned(),
        "agent_team_members" => *status = "pending".to_owned(),
        "agent_team_tasks" => *status = "todo".to_owned(),
        "agent_runs" => bail!("unknown legacy agent-run status `{status}`"),
        _ => {}
    }
    Ok(())
}

fn sql_value(value: ValueRef<'_>) -> Result<Value> {
    Ok(match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(value) => Value::Number(Number::from(value)),
        ValueRef::Real(value) => Number::from_f64(value)
            .map(Value::Number)
            .context("legacy SQLite value is not a finite number")?,
        ValueRef::Text(value) => Value::String(
            std::str::from_utf8(value)
                .context("legacy SQLite text is not UTF-8")?
                .to_owned(),
        ),
        ValueRef::Blob(_) => bail!("unexpected binary value in legacy session row"),
    })
}

fn table_exists(connection: &rusqlite::Connection, table: &str) -> Result<bool> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [table],
            |row| row.get(0),
        )
        .with_context(|| format!("inspect legacy table `{table}`"))
}

fn table_columns(
    connection: &rusqlite::Connection,
    table: &str,
) -> Result<std::collections::HashSet<String>> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info(\"{table}\")"))
        .with_context(|| format!("inspect legacy `{table}` columns"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .with_context(|| format!("read legacy `{table}` columns"))?
        .collect::<rusqlite::Result<std::collections::HashSet<_>>>()
        .with_context(|| format!("collect legacy `{table}` columns"))?;
    Ok(columns)
}

#[cfg(test)]
#[path = "session_db_tests.rs"]
mod tests;
