use anyhow::{Context, Result};
use rusqlite::{params, types::Type, Row, ToSql};
use serde_json::Value;

use crate::openhuman::config::Config;
use crate::openhuman::memory_tree;

use super::types::{McpWriteListQuery, McpWriteRecord, NewMcpWriteRecord};

const DEFAULT_LIST_LIMIT: u64 = 50;
const MAX_LIST_LIMIT: u64 = 500;

pub fn record_write(config: &Config, record: NewMcpWriteRecord) -> Result<i64> {
    let args_summary = serde_json::to_string(&record.args_summary)
        .context("failed to serialize mcp write args_summary")?;
    memory_tree::store::with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO mcp_writes (
                timestamp_ms,
                client_info,
                tool_name,
                args_summary,
                resulting_chunk_id,
                success,
                error_message
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                record.timestamp_ms,
                record.client_info,
                record.tool_name,
                args_summary,
                record.resulting_chunk_id,
                if record.success { 1_i64 } else { 0_i64 },
                record.error_message,
            ],
        )
        .context("failed to insert mcp_writes audit row")?;
        Ok(conn.last_insert_rowid())
    })
}

pub fn list_writes(config: &Config, query: &McpWriteListQuery) -> Result<Vec<McpWriteRecord>> {
    memory_tree::store::with_connection(config, |conn| {
        let mut sql = String::from(
            "SELECT
                id,
                timestamp_ms,
                client_info,
                tool_name,
                args_summary,
                resulting_chunk_id,
                success,
                error_message
             FROM mcp_writes
             WHERE 1=1",
        );
        let mut bound: Vec<Box<dyn ToSql>> = Vec::new();

        if let Some(since_ms) = query.since_ms {
            sql.push_str(" AND timestamp_ms >= ?");
            bound.push(Box::new(u64_to_i64(since_ms, "since_ms")?));
        }
        if let Some(client) = normalized_filter(query.client_filter.as_deref()) {
            sql.push_str(" AND client_info = ?");
            bound.push(Box::new(client.to_string()));
        }
        if let Some(tool) = normalized_filter(query.tool_filter.as_deref()) {
            sql.push_str(" AND tool_name = ?");
            bound.push(Box::new(tool.to_string()));
        }
        if query.success_only.unwrap_or(false) {
            sql.push_str(" AND success = 1");
        }

        sql.push_str(" ORDER BY timestamp_ms DESC, id DESC LIMIT ? OFFSET ?");
        bound.push(Box::new(normalized_limit(query.limit)?));
        bound.push(Box::new(normalized_offset(query.offset)?));

        let refs = bound
            .iter()
            .map(|value| value.as_ref() as &dyn ToSql)
            .collect::<Vec<_>>();
        let mut stmt = conn
            .prepare(&sql)
            .context("failed to prepare mcp_writes list query")?;
        let rows = stmt
            .query_map(refs.as_slice(), row_to_record)
            .context("failed to query mcp_writes")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to collect mcp_writes rows")?;
        Ok(rows)
    })
}

fn normalized_limit(limit: Option<u64>) -> Result<i64> {
    u64_to_i64(
        limit.unwrap_or(DEFAULT_LIST_LIMIT).min(MAX_LIST_LIMIT),
        "limit",
    )
}

fn normalized_offset(offset: Option<u64>) -> Result<i64> {
    u64_to_i64(offset.unwrap_or(0), "offset")
}

fn u64_to_i64(value: u64, field: &str) -> Result<i64> {
    i64::try_from(value).with_context(|| format!("{field} is too large for SQLite INTEGER"))
}

fn normalized_filter(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn row_to_record(row: &Row<'_>) -> rusqlite::Result<McpWriteRecord> {
    let args_summary_text: Option<String> = row.get(4)?;
    let args_summary = match args_summary_text {
        Some(text) => serde_json::from_str::<Value>(&text).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(err))
        })?,
        None => Value::Null,
    };
    let success: i64 = row.get(6)?;
    Ok(McpWriteRecord {
        id: row.get(0)?,
        timestamp_ms: row.get(1)?,
        client_info: row.get(2)?,
        tool_name: row.get(3)?,
        args_summary,
        resulting_chunk_id: row.get(5)?,
        success: success != 0,
        error_message: row.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn test_config(tmp: &TempDir) -> Config {
        let mut config = Config::default();
        config.workspace_dir = tmp.path().join("workspace");
        std::fs::create_dir_all(&config.workspace_dir).unwrap();
        config
    }

    fn record(
        timestamp_ms: i64,
        client_info: &str,
        tool_name: &str,
        success: bool,
    ) -> NewMcpWriteRecord {
        NewMcpWriteRecord {
            timestamp_ms,
            client_info: client_info.to_string(),
            tool_name: tool_name.to_string(),
            args_summary: json!({ "title": format!("record-{timestamp_ms}") }),
            resulting_chunk_id: success.then(|| format!("chunk-{timestamp_ms}")),
            success,
            error_message: (!success).then(|| "write failed".to_string()),
        }
    }

    #[test]
    fn record_write_inserts_success_and_failure_rows() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);

        let success_id = record_write(
            &config,
            record(100, "mcp:claude-desktop", "memory.store", true),
        )
        .unwrap();
        let failure_id =
            record_write(&config, record(200, "mcp:cursor", "tree.tag", false)).unwrap();

        let rows = list_writes(&config, &McpWriteListQuery::default()).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, failure_id);
        assert!(!rows[0].success);
        assert_eq!(rows[0].error_message.as_deref(), Some("write failed"));
        assert_eq!(rows[1].id, success_id);
        assert!(rows[1].success);
        assert_eq!(rows[1].resulting_chunk_id.as_deref(), Some("chunk-100"));
    }

    #[test]
    fn list_writes_filters_by_client_tool_since_and_success() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        for row in [
            record(100, "mcp:claude-desktop", "memory.store", true),
            record(200, "mcp:cursor", "memory.note", false),
            record(300, "mcp:claude-desktop", "tree.tag", true),
            record(400, "mcp:cursor", "tree.tag", true),
        ] {
            record_write(&config, row).unwrap();
        }

        let by_client = list_writes(
            &config,
            &McpWriteListQuery {
                client_filter: Some("mcp:claude-desktop".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(by_client.len(), 2);
        assert!(by_client
            .iter()
            .all(|row| row.client_info == "mcp:claude-desktop"));

        let by_tool = list_writes(
            &config,
            &McpWriteListQuery {
                tool_filter: Some("tree.tag".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            by_tool
                .iter()
                .map(|row| row.timestamp_ms)
                .collect::<Vec<_>>(),
            vec![400, 300]
        );

        let since = list_writes(
            &config,
            &McpWriteListQuery {
                since_ms: Some(250),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            since.iter().map(|row| row.timestamp_ms).collect::<Vec<_>>(),
            vec![400, 300]
        );

        let success_only = list_writes(
            &config,
            &McpWriteListQuery {
                success_only: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(success_only.len(), 3);
        assert!(success_only.iter().all(|row| row.success));
    }

    #[test]
    fn list_writes_orders_newest_first_and_supports_limit_offset() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        for ts in [100, 200, 300, 400] {
            record_write(&config, record(ts, "mcp", "memory.store", true)).unwrap();
        }

        let rows = list_writes(
            &config,
            &McpWriteListQuery {
                limit: Some(2),
                offset: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            rows.iter().map(|row| row.timestamp_ms).collect::<Vec<_>>(),
            vec![300, 200]
        );
    }

    #[test]
    fn list_writes_caps_limit_at_max() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        for ts in 0..505 {
            record_write(&config, record(ts, "mcp", "memory.store", true)).unwrap();
        }

        let rows = list_writes(
            &config,
            &McpWriteListQuery {
                limit: Some(MAX_LIST_LIMIT + 100),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(rows.len(), MAX_LIST_LIMIT as usize);
    }
}
