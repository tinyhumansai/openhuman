//! Row conversion for the SQLite task-source store.

use crate::storage::rusqlite;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use super::super::types::{FilterSpec, ProviderSlug, SourceTarget, TaskSource};

pub(super) const SELECT_SOURCE_COLUMNS: &str =
    "SELECT id, provider, connection_id, name, enabled, filter, \
     interval_secs, target, max_tasks_per_fetch, created_at, last_fetch_at, last_status \
     FROM task_sources";

pub(super) fn map_source_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskSource> {
    let provider_raw: String = row.get(1)?;
    let provider = ProviderSlug::parse(&provider_raw).map_err(sql_conv)?;

    let filter_raw: String = row.get(5)?;
    let filter: FilterSpec = serde_json::from_str(&filter_raw)
        .map_err(|e| sql_conv(format!("invalid filter json: {e}")))?;

    let target_raw: String = row.get(7)?;
    let target: SourceTarget = serde_json::from_str(&target_raw)
        .map_err(|e| sql_conv(format!("invalid target json: {e}")))?;

    let created_at_raw: String = row.get(9)?;
    let last_fetch_raw: Option<String> = row.get(10)?;

    Ok(TaskSource {
        id: row.get(0)?,
        provider,
        connection_id: row.get(2)?,
        name: row.get(3)?,
        enabled: row.get::<_, i64>(4)? != 0,
        filter,
        interval_secs: u64::try_from(row.get::<_, i64>(6)?)
            .map_err(|_| sql_conv("invalid negative interval_secs in task_sources DB"))?,
        target,
        max_tasks_per_fetch: u32::try_from(row.get::<_, i64>(8)?)
            .map_err(|_| sql_conv("invalid max_tasks_per_fetch in task_sources DB"))?,
        created_at: parse_rfc3339(&created_at_raw).map_err(sql_conv)?,
        last_fetch_at: match last_fetch_raw {
            Some(raw) => Some(parse_rfc3339(&raw).map_err(sql_conv)?),
            None => None,
        },
        last_status: row.get(11)?,
    })
}

fn parse_rfc3339(raw: &str) -> Result<DateTime<Utc>> {
    let parsed = DateTime::parse_from_rfc3339(raw)
        .with_context(|| format!("Invalid RFC3339 timestamp in task_sources DB: {raw}"))?;
    Ok(parsed.with_timezone(&Utc))
}

fn sql_conv<E: std::fmt::Display>(err: E) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(anyhow::anyhow!("{err}").into())
}
