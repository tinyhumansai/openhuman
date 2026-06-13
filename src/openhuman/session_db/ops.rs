use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};

use crate::openhuman::config::Config;

use super::store::with_connection;
use super::types::{
    SessionMessage, SessionRecord, SessionSearchParams, SessionSearchResult, SessionStatus,
    SessionToolCall,
};

const MAX_TOOL_OUTPUT_BYTES: usize = 32 * 1024;

pub fn record_session_start(
    config: &Config,
    id: &str,
    agent_definition_id: &str,
    agent_definition_name: &str,
    session_key: &str,
    parent_session_id: Option<&str>,
    thread_id: Option<&str>,
    source_channel: Option<&str>,
    model: Option<&str>,
    transcript_path: Option<&str>,
) -> Result<SessionRecord> {
    let now = Utc::now();
    log::debug!(
        "[session_db] record_session_start id={id} agent={agent_definition_id} \
         parent={} thread={} channel={}",
        parent_session_id.unwrap_or("-"),
        thread_id.unwrap_or("-"),
        source_channel.unwrap_or("-"),
    );

    with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO sessions (
                id, agent_definition_id, agent_definition_name, session_key,
                parent_session_id, thread_id, source_channel, status, model,
                transcript_path, started_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'running', ?8, ?9, ?10)",
            params![
                id,
                agent_definition_id,
                agent_definition_name,
                session_key,
                parent_session_id,
                thread_id,
                source_channel,
                model,
                transcript_path,
                now.to_rfc3339(),
            ],
        )
        .context("failed to insert session")?;

        index_fts_session(conn, id, agent_definition_name)?;
        Ok(())
    })?;

    get_session(config, id)
}

pub fn record_session_end(
    config: &Config,
    id: &str,
    status: SessionStatus,
    turn_count: u32,
    input_tokens: u64,
    output_tokens: u64,
    cached_input_tokens: u64,
    cost_usd: f64,
) -> Result<SessionRecord> {
    let now = Utc::now();
    log::debug!(
        "[session_db] record_session_end id={id} status={} turns={turn_count} \
         tokens_in={input_tokens} tokens_out={output_tokens} cost=${cost_usd:.6}",
        status.as_str(),
    );

    with_connection(config, |conn| {
        conn.execute(
            "UPDATE sessions SET
                status = ?1, turn_count = ?2, input_tokens = ?3,
                output_tokens = ?4, cached_input_tokens = ?5,
                cost_usd = ?6, ended_at = ?7
             WHERE id = ?8",
            params![
                status.as_str(),
                turn_count,
                input_tokens as i64,
                output_tokens as i64,
                cached_input_tokens as i64,
                cost_usd,
                now.to_rfc3339(),
                id,
            ],
        )
        .context("failed to update session end")?;
        Ok(())
    })?;

    get_session(config, id)
}

pub fn record_message(
    config: &Config,
    session_id: &str,
    role: &str,
    content: &str,
    model: Option<&str>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cost_usd: Option<f64>,
) -> Result<i64> {
    let now = Utc::now();
    log::trace!(
        "[session_db] record_message session={session_id} role={role} len={}",
        content.len()
    );

    with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO session_messages (
                session_id, role, content, model,
                input_tokens, output_tokens, cost_usd, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                session_id,
                role,
                content,
                model,
                input_tokens.map(|v| v as i64),
                output_tokens.map(|v| v as i64),
                cost_usd,
                now.to_rfc3339(),
            ],
        )
        .context("failed to insert session message")?;

        let msg_id = conn.last_insert_rowid();

        index_fts_content(conn, session_id, content)?;

        Ok(msg_id)
    })
}

pub fn record_tool_call(
    config: &Config,
    session_id: &str,
    message_id: Option<i64>,
    tool_name: &str,
    tool_input: Option<&str>,
    tool_output: Option<&str>,
    status: &str,
    duration_ms: Option<i64>,
) -> Result<i64> {
    let now = Utc::now();
    log::trace!(
        "[session_db] record_tool_call session={session_id} tool={tool_name} status={status}"
    );

    let bounded_output = tool_output.map(|o| {
        if o.len() <= MAX_TOOL_OUTPUT_BYTES {
            o.to_string()
        } else {
            let mut cutoff = MAX_TOOL_OUTPUT_BYTES;
            while cutoff > 0 && !o.is_char_boundary(cutoff) {
                cutoff -= 1;
            }
            let mut truncated = o[..cutoff].to_string();
            truncated.push_str("\n...[truncated]");
            truncated
        }
    });

    with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO session_tool_calls (
                session_id, message_id, tool_name, tool_input,
                tool_output, status, duration_ms, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                session_id,
                message_id,
                tool_name,
                tool_input,
                bounded_output,
                status,
                duration_ms,
                now.to_rfc3339(),
            ],
        )
        .context("failed to insert tool call")?;

        index_fts_tool(conn, session_id, tool_name)?;

        Ok(conn.last_insert_rowid())
    })
}

pub fn get_session(config: &Config, id: &str) -> Result<SessionRecord> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, agent_definition_id, agent_definition_name, session_key,
                    parent_session_id, thread_id, source_channel, status, model,
                    turn_count, input_tokens, output_tokens, cached_input_tokens,
                    cost_usd, transcript_path, started_at, ended_at
             FROM sessions WHERE id = ?1",
        )?;

        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            map_session_row(row).map_err(Into::into)
        } else {
            anyhow::bail!("session '{id}' not found")
        }
    })
}

pub fn list_sessions(
    config: &Config,
    limit: Option<u32>,
    offset: Option<u32>,
    status: Option<&str>,
    parent_id: Option<&str>,
) -> Result<SessionSearchResult> {
    log::debug!(
        "[session_db] list_sessions limit={} offset={} status={} parent={}",
        limit.unwrap_or(50),
        offset.unwrap_or(0),
        status.unwrap_or("-"),
        parent_id.unwrap_or("-"),
    );

    with_connection(config, |conn| {
        let mut where_clauses: Vec<String> = Vec::new();
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(s) = status {
            param_values.push(Box::new(s.to_string()));
            where_clauses.push(format!("status = ?{}", param_values.len()));
        }
        if let Some(p) = parent_id {
            param_values.push(Box::new(p.to_string()));
            where_clauses.push(format!("parent_session_id = ?{}", param_values.len()));
        }

        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_clauses.join(" AND "))
        };

        let lim = limit.unwrap_or(50).min(500) as i64;
        let off = offset.unwrap_or(0) as i64;

        let count_sql = format!("SELECT COUNT(*) FROM sessions {where_sql}");
        let total: u64 = {
            let mut stmt = conn.prepare(&count_sql)?;
            let params_ref: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(|b| b.as_ref()).collect();
            stmt.query_row(params_ref.as_slice(), |r| r.get::<_, i64>(0))? as u64
        };

        param_values.push(Box::new(lim));
        let lim_idx = param_values.len();
        param_values.push(Box::new(off));
        let off_idx = param_values.len();

        let query_sql = format!(
            "SELECT id, agent_definition_id, agent_definition_name, session_key,
                    parent_session_id, thread_id, source_channel, status, model,
                    turn_count, input_tokens, output_tokens, cached_input_tokens,
                    cost_usd, transcript_path, started_at, ended_at
             FROM sessions {where_sql}
             ORDER BY started_at DESC
             LIMIT ?{lim_idx} OFFSET ?{off_idx}",
        );

        let mut stmt = conn.prepare(&query_sql)?;
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|b| b.as_ref()).collect();
        let rows = stmt.query_map(params_ref.as_slice(), map_session_row)?;

        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }

        Ok(SessionSearchResult { sessions, total })
    })
}

pub fn search_sessions(
    config: &Config,
    params: &SessionSearchParams,
) -> Result<SessionSearchResult> {
    log::debug!(
        "[session_db] search_sessions query={} agent={} tool={} channel={} thread={}",
        params.query.as_deref().unwrap_or("-"),
        params.agent_id.as_deref().unwrap_or("-"),
        params.tool_name.as_deref().unwrap_or("-"),
        params.source_channel.as_deref().unwrap_or("-"),
        params.thread_id.as_deref().unwrap_or("-"),
    );

    with_connection(config, |conn| search_sessions_inner(conn, params))
}

fn search_sessions_inner(
    conn: &Connection,
    params: &SessionSearchParams,
) -> Result<SessionSearchResult> {
    let lim = params.limit.unwrap_or(50).min(500) as i64;
    let off = params.offset.unwrap_or(0) as i64;

    let mut where_clauses: Vec<String> = Vec::new();
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(ref q) = params.query {
        if !q.trim().is_empty() {
            param_values.push(Box::new(q.clone()));
            where_clauses.push(format!(
                "s.id IN (SELECT session_id FROM sessions_fts WHERE sessions_fts MATCH ?{})",
                param_values.len()
            ));
        }
    }

    if let Some(ref agent) = params.agent_id {
        param_values.push(Box::new(agent.clone()));
        where_clauses.push(format!("s.agent_definition_id = ?{}", param_values.len()));
    }

    if let Some(ref tool) = params.tool_name {
        param_values.push(Box::new(tool.clone()));
        where_clauses.push(format!(
            "s.id IN (SELECT DISTINCT session_id FROM session_tool_calls WHERE tool_name = ?{})",
            param_values.len()
        ));
    }

    if let Some(ref channel) = params.source_channel {
        param_values.push(Box::new(channel.clone()));
        where_clauses.push(format!("s.source_channel = ?{}", param_values.len()));
    }

    if let Some(ref parent) = params.parent_session_id {
        param_values.push(Box::new(parent.clone()));
        where_clauses.push(format!("s.parent_session_id = ?{}", param_values.len()));
    }

    if let Some(ref status) = params.status {
        param_values.push(Box::new(status.clone()));
        where_clauses.push(format!("s.status = ?{}", param_values.len()));
    }

    if let Some(ref tid) = params.thread_id {
        param_values.push(Box::new(tid.clone()));
        where_clauses.push(format!("s.thread_id = ?{}", param_values.len()));
    }

    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_clauses.join(" AND "))
    };

    let count_sql = format!("SELECT COUNT(*) FROM sessions s {where_sql}");
    let total: u64 = {
        let mut stmt = conn.prepare(&count_sql)?;
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|b| b.as_ref()).collect();
        stmt.query_row(params_ref.as_slice(), |r| r.get::<_, i64>(0))? as u64
    };

    param_values.push(Box::new(lim));
    let lim_idx = param_values.len();
    param_values.push(Box::new(off));
    let off_idx = param_values.len();

    let query = format!(
        "SELECT s.id, s.agent_definition_id, s.agent_definition_name, s.session_key,
                s.parent_session_id, s.thread_id, s.source_channel, s.status, s.model,
                s.turn_count, s.input_tokens, s.output_tokens, s.cached_input_tokens,
                s.cost_usd, s.transcript_path, s.started_at, s.ended_at
         FROM sessions s {where_sql}
         ORDER BY s.started_at DESC
         LIMIT ?{lim_idx} OFFSET ?{off_idx}",
    );

    let mut stmt = conn.prepare(&query)?;
    let params_ref: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(|b| b.as_ref()).collect();
    let rows = stmt.query_map(params_ref.as_slice(), map_session_row)?;

    let mut sessions = Vec::new();
    for row in rows {
        sessions.push(row?);
    }

    Ok(SessionSearchResult { sessions, total })
}

pub fn import_transcript(
    config: &Config,
    transcript_path: &std::path::Path,
) -> Result<SessionRecord> {
    use crate::openhuman::agent::harness::session::transcript::read_transcript;

    let transcript = read_transcript(transcript_path)
        .with_context(|| format!("failed to read transcript {}", transcript_path.display()))?;

    let stem = transcript_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    let session_key = stem.to_string();
    let id = uuid::Uuid::new_v4().to_string();
    let meta = &transcript.meta;

    let parent_session_id: Option<&str> = None;
    let is_subagent = stem.contains("__");
    let parent_prefix = if is_subagent {
        stem.rsplit_once("__").map(|(p, _)| p)
    } else {
        None
    };

    log::debug!(
        "[session_db] import_transcript path={} stem={stem} agent={} turns={} cost=${:.6}",
        transcript_path.display(),
        meta.agent_name,
        meta.turn_count,
        meta.charged_amount_usd,
    );

    with_connection(config, |conn| {
        let parent_id = if let Some(prefix) = parent_prefix {
            let mut stmt = conn.prepare(
                "SELECT id FROM sessions WHERE session_key = ?1 ORDER BY started_at DESC LIMIT 1",
            )?;
            let mut rows = stmt.query(params![prefix])?;
            rows.next()?.map(|r| r.get::<_, String>(0)).transpose()?
        } else {
            None
        };

        let started = chrono::DateTime::parse_from_rfc3339(&meta.created)
            .unwrap_or_else(|_| Utc::now().into())
            .with_timezone(&Utc);
        let ended = chrono::DateTime::parse_from_rfc3339(&meta.updated)
            .ok()
            .map(|dt| dt.with_timezone(&Utc));

        conn.execute(
            "INSERT OR IGNORE INTO sessions (
                id, agent_definition_id, agent_definition_name, session_key,
                parent_session_id, thread_id, source_channel, status, model,
                turn_count, input_tokens, output_tokens, cached_input_tokens,
                cost_usd, transcript_path, started_at, ended_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                id,
                meta.agent_name,
                meta.agent_name,
                session_key,
                parent_id.as_deref().or(parent_session_id),
                meta.thread_id,
                meta.dispatcher,
                "completed",
                Option::<String>::None,
                meta.turn_count as i64,
                meta.input_tokens as i64,
                meta.output_tokens as i64,
                meta.cached_input_tokens as i64,
                meta.charged_amount_usd,
                transcript_path.to_string_lossy().as_ref(),
                started.to_rfc3339(),
                ended.map(|dt| dt.to_rfc3339()),
            ],
        )
        .context("failed to insert imported session")?;

        index_fts_session(conn, &id, &meta.agent_name)?;

        for msg in &transcript.messages {
            conn.execute(
                "INSERT INTO session_messages (session_id, role, content, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![id, msg.role, msg.content, started.to_rfc3339()],
            )?;
            index_fts_content(conn, &id, &msg.content)?;
        }

        Ok(())
    })?;

    get_session(config, &id)
}

pub fn list_messages(
    config: &Config,
    session_id: &str,
    limit: Option<u32>,
) -> Result<Vec<SessionMessage>> {
    with_connection(config, |conn| {
        let lim = limit.unwrap_or(200).min(1000) as i64;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, model,
                    input_tokens, output_tokens, cost_usd, created_at
             FROM session_messages
             WHERE session_id = ?1
             ORDER BY id ASC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![session_id, lim], |row| {
            Ok(SessionMessage {
                id: row.get(0)?,
                session_id: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                model: row.get(4)?,
                input_tokens: row.get::<_, Option<i64>>(5)?.map(|v| v as u64),
                output_tokens: row.get::<_, Option<i64>>(6)?.map(|v| v as u64),
                cost_usd: row.get(7)?,
                created_at: parse_rfc3339(&row.get::<_, String>(8)?)
                    .map_err(sql_conversion_error)?,
            })
        })?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        Ok(messages)
    })
}

pub fn list_tool_calls(
    config: &Config,
    session_id: &str,
    limit: Option<u32>,
) -> Result<Vec<SessionToolCall>> {
    with_connection(config, |conn| {
        let lim = limit.unwrap_or(200).min(1000) as i64;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, message_id, tool_name, tool_input,
                    tool_output, status, duration_ms, created_at
             FROM session_tool_calls
             WHERE session_id = ?1
             ORDER BY id ASC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![session_id, lim], |row| {
            Ok(SessionToolCall {
                id: row.get(0)?,
                session_id: row.get(1)?,
                message_id: row.get(2)?,
                tool_name: row.get(3)?,
                tool_input: row.get(4)?,
                tool_output: row.get(5)?,
                status: row.get(6)?,
                duration_ms: row.get(7)?,
                created_at: parse_rfc3339(&row.get::<_, String>(8)?)
                    .map_err(sql_conversion_error)?,
            })
        })?;

        let mut tool_calls = Vec::new();
        for row in rows {
            tool_calls.push(row?);
        }
        Ok(tool_calls)
    })
}

pub fn list_children(config: &Config, session_id: &str) -> Result<Vec<SessionRecord>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, agent_definition_id, agent_definition_name, session_key,
                    parent_session_id, thread_id, source_channel, status, model,
                    turn_count, input_tokens, output_tokens, cached_input_tokens,
                    cost_usd, transcript_path, started_at, ended_at
             FROM sessions
             WHERE parent_session_id = ?1
             ORDER BY started_at ASC",
        )?;

        let rows = stmt.query_map(params![session_id], map_session_row)?;
        let mut children = Vec::new();
        for row in rows {
            children.push(row?);
        }
        Ok(children)
    })
}

pub fn mark_interrupted(config: &Config) -> Result<usize> {
    log::debug!("[session_db] mark_interrupted — marking all running sessions as interrupted");
    with_connection(config, |conn| {
        let now = Utc::now();
        let changed = conn.execute(
            "UPDATE sessions SET status = 'interrupted', ended_at = ?1
             WHERE status = 'running'",
            params![now.to_rfc3339()],
        )?;
        if changed > 0 {
            log::info!("[session_db] marked {changed} running session(s) as interrupted");
        }
        Ok(changed)
    })
}

fn index_fts_session(conn: &Connection, session_id: &str, agent_name: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO sessions_fts (session_id, agent_definition_name, content, tool_name)
         VALUES (?1, ?2, '', '')",
        params![session_id, agent_name],
    )
    .context("failed to index session in FTS")?;
    Ok(())
}

fn index_fts_content(conn: &Connection, session_id: &str, content: &str) -> Result<()> {
    let snippet = if content.len() > 2000 {
        &content[..2000]
    } else {
        content
    };
    conn.execute(
        "INSERT INTO sessions_fts (session_id, agent_definition_name, content, tool_name)
         VALUES (?1, '', ?2, '')",
        params![session_id, snippet],
    )
    .context("failed to index content in FTS")?;
    Ok(())
}

fn index_fts_tool(conn: &Connection, session_id: &str, tool_name: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO sessions_fts (session_id, agent_definition_name, content, tool_name)
         VALUES (?1, '', '', ?2)",
        params![session_id, tool_name],
    )
    .context("failed to index tool call in FTS")?;
    Ok(())
}

fn map_session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRecord> {
    let started_at_raw: String = row.get(15)?;
    let ended_at_raw: Option<String> = row.get(16)?;

    Ok(SessionRecord {
        id: row.get(0)?,
        agent_definition_id: row.get(1)?,
        agent_definition_name: row.get(2)?,
        session_key: row.get(3)?,
        parent_session_id: row.get(4)?,
        thread_id: row.get(5)?,
        source_channel: row.get(6)?,
        status: SessionStatus::parse(&row.get::<_, String>(7)?),
        model: row.get(8)?,
        turn_count: row.get::<_, i64>(9)? as u32,
        input_tokens: row.get::<_, i64>(10)? as u64,
        output_tokens: row.get::<_, i64>(11)? as u64,
        cached_input_tokens: row.get::<_, i64>(12)? as u64,
        cost_usd: row.get(13)?,
        transcript_path: row.get(14)?,
        started_at: parse_rfc3339(&started_at_raw).map_err(sql_conversion_error)?,
        ended_at: match ended_at_raw {
            Some(raw) => Some(parse_rfc3339(&raw).map_err(sql_conversion_error)?),
            None => None,
        },
    })
}

fn parse_rfc3339(raw: &str) -> Result<DateTime<Utc>> {
    let parsed = DateTime::parse_from_rfc3339(raw)
        .with_context(|| format!("invalid RFC3339 timestamp in session DB: {raw}"))?;
    Ok(parsed.with_timezone(&Utc))
}

fn sql_conversion_error(err: anyhow::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(err.into())
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
