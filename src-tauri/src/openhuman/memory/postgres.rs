use super::traits::{Memory, MemoryCategory, MemoryEntry};
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use postgres::{Client, NoTls, Row};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// Maximum allowed connect timeout (seconds) to avoid unreasonable waits.
const POSTGRES_CONNECT_TIMEOUT_CAP_SECS: u64 = 300;

/// PostgreSQL-backed persistent memory.
///
/// This backend focuses on reliable CRUD and keyword recall using SQL, without
/// requiring extension setup (for example pgvector).
pub struct PostgresMemory {
    client: Arc<Mutex<Client>>,
    qualified_table: String,
}

impl PostgresMemory {
    pub fn new(
        db_url: &str,
        schema: &str,
        table: &str,
        connect_timeout_secs: Option<u64>,
    ) -> Result<Self> {
        validate_identifier(schema, "storage schema")?;
        validate_identifier(table, "storage table")?;

        let mut config: postgres::Config = db_url
            .parse()
            .context("invalid PostgreSQL connection URL")?;

        if let Some(timeout_secs) = connect_timeout_secs {
            let bounded = timeout_secs.min(POSTGRES_CONNECT_TIMEOUT_CAP_SECS);
            config.connect_timeout(Duration::from_secs(bounded));
        }

        let mut client = config
            .connect(NoTls)
            .context("failed to connect to PostgreSQL memory backend")?;

        let schema_ident = quote_identifier(schema);
        let table_ident = quote_identifier(table);
        let qualified_table = format!("{schema_ident}.{table_ident}");

        Self::init_schema(&mut client, &schema_ident, &qualified_table)?;

        Ok(Self {
            client: Arc::new(Mutex::new(client)),
            qualified_table,
        })
    }

    fn init_schema(client: &mut Client, schema_ident: &str, qualified_table: &str) -> Result<()> {
        client.batch_execute(&format!(
            "
            CREATE SCHEMA IF NOT EXISTS {schema_ident};

            CREATE TABLE IF NOT EXISTS {qualified_table} (
                id TEXT PRIMARY KEY,
                key TEXT UNIQUE NOT NULL,
                content TEXT NOT NULL,
                category TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL,
                updated_at TIMESTAMPTZ NOT NULL,
                session_id TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_memories_category ON {qualified_table}(category);
            CREATE INDEX IF NOT EXISTS idx_memories_session_id ON {qualified_table}(session_id);
            CREATE INDEX IF NOT EXISTS idx_memories_updated_at ON {qualified_table}(updated_at DESC);
            "
        ))?;

        Ok(())
    }

    fn category_to_str(category: &MemoryCategory) -> String {
        match category {
            MemoryCategory::Core => "core".to_string(),
            MemoryCategory::Daily => "daily".to_string(),
            MemoryCategory::Conversation => "conversation".to_string(),
            MemoryCategory::Custom(name) => name.clone(),
        }
    }

    fn parse_category(value: &str) -> MemoryCategory {
        match value {
            "core" => MemoryCategory::Core,
            "daily" => MemoryCategory::Daily,
            "conversation" => MemoryCategory::Conversation,
            other => MemoryCategory::Custom(other.to_string()),
        }
    }

    fn row_to_entry(row: &Row) -> Result<MemoryEntry> {
        let timestamp: DateTime<Utc> = row.get(4);

        Ok(MemoryEntry {
            id: row.get(0),
            key: row.get(1),
            content: row.get(2),
            category: Self::parse_category(&row.get::<_, String>(3)),
            timestamp: timestamp.to_rfc3339(),
            session_id: row.get(5),
            score: row.try_get(6).ok(),
        })
    }
}

fn validate_identifier(value: &str, field_name: &str) -> Result<()> {
    if value.is_empty() {
        anyhow::bail!("{field_name} must not be empty");
    }

    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        anyhow::bail!("{field_name} must not be empty");
    };

    if !(first.is_ascii_alphabetic() || first == '_') {
        anyhow::bail!("{field_name} must start with an ASCII letter or underscore; got '{value}'");
    }

    if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        anyhow::bail!(
            "{field_name} can only contain ASCII letters, numbers, and underscores; got '{value}'"
        );
    }

    Ok(())
}

fn quote_identifier(value: &str) -> String {
    format!("\"{value}\"")
}

#[async_trait]
impl Memory for PostgresMemory {
    fn name(&self) -> &str {
        "postgres"
    }

    async fn store(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> Result<()> {
        let client = self.client.clone();
        let qualified_table = self.qualified_table.clone();
        let key = key.to_string();
        let content = content.to_string();
        let category = Self::category_to_str(&category);
        let sid = session_id.map(str::to_string);

        tokio::task::spawn_blocking(move || -> Result<()> {
            let now = Utc::now();
            let mut client = client.lock();
            let stmt = format!(
                "
                INSERT INTO {qualified_table}
                    (id, key, content, category, created_at, updated_at, session_id)
                VALUES
                    ($1, $2, $3, $4, $5, $6, $7)
                ON CONFLICT (key) DO UPDATE SET
                    content = EXCLUDED.content,
                    category = EXCLUDED.category,
                    updated_at = EXCLUDED.updated_at,
                    session_id = EXCLUDED.session_id
                "
            );

            let id = Uuid::new_v4().to_string();
            client.execute(&stmt, &[&id, &key, &content, &category, &now, &now, &sid])?;
            Ok(())
        })
        .await?
    }

    async fn recall(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        let client = self.client.clone();
        let qualified_table = self.qualified_table.clone();
        let query = query.trim().to_string();
        let sid = session_id.map(str::to_string);

        tokio::task::spawn_blocking(move || -> Result<Vec<MemoryEntry>> {
            let mut client = client.lock();
            let stmt = format!(
                "
                SELECT id, key, content, category, created_at, session_id,
                       (
                         CASE WHEN key ILIKE '%' || $1 || '%' THEN 2.0 ELSE 0.0 END +
                         CASE WHEN content ILIKE '%' || $1 || '%' THEN 1.0 ELSE 0.0 END
                       ) AS score
                FROM {qualified_table}
                WHERE ($2::TEXT IS NULL OR session_id = $2)
                  AND ($1 = '' OR key ILIKE '%' || $1 || '%' OR content ILIKE '%' || $1 || '%')
                ORDER BY score DESC, updated_at DESC
                LIMIT $3
                "
            );

            #[allow(clippy::cast_possible_wrap)]
            let limit_i64 = limit as i64;

            let rows = client.query(&stmt, &[&query, &sid, &limit_i64])?;
            rows.iter()
                .map(Self::row_to_entry)
                .collect::<Result<Vec<MemoryEntry>>>()
        })
        .await?
    }

    async fn get(&self, key: &str) -> Result<Option<MemoryEntry>> {
        let client = self.client.clone();
        let qualified_table = self.qualified_table.clone();
        let key = key.to_string();

        tokio::task::spawn_blocking(move || -> Result<Option<MemoryEntry>> {
            let mut client = client.lock();
            let stmt = format!(
                "
                SELECT id, key, content, category, created_at, session_id
                FROM {qualified_table}
                WHERE key = $1
                LIMIT 1
                "
            );

            let row = client.query_opt(&stmt, &[&key])?;
            row.as_ref().map(Self::row_to_entry).transpose()
        })
        .await?
    }

    async fn list(
        &self,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        let client = self.client.clone();
        let qualified_table = self.qualified_table.clone();
        let category = category.map(Self::category_to_str);
        let sid = session_id.map(str::to_string);

        tokio::task::spawn_blocking(move || -> Result<Vec<MemoryEntry>> {
            let mut client = client.lock();
            let stmt = format!(
                "
                SELECT id, key, content, category, created_at, session_id
                FROM {qualified_table}
                WHERE ($1::TEXT IS NULL OR category = $1)
                  AND ($2::TEXT IS NULL OR session_id = $2)
                ORDER BY updated_at DESC
                "
            );

            let category_ref = category.as_deref();
            let session_ref = sid.as_deref();
            let rows = client.query(&stmt, &[&category_ref, &session_ref])?;
            rows.iter()
                .map(Self::row_to_entry)
                .collect::<Result<Vec<MemoryEntry>>>()
        })
        .await?
    }

    async fn forget(&self, key: &str) -> Result<bool> {
        let client = self.client.clone();
        let qualified_table = self.qualified_table.clone();
        let key = key.to_string();

        tokio::task::spawn_blocking(move || -> Result<bool> {
            let mut client = client.lock();
            let stmt = format!("DELETE FROM {qualified_table} WHERE key = $1");
            let deleted = client.execute(&stmt, &[&key])?;
            Ok(deleted > 0)
        })
        .await?
    }

    async fn count(&self) -> Result<usize> {
        let client = self.client.clone();
        let qualified_table = self.qualified_table.clone();

        tokio::task::spawn_blocking(move || -> Result<usize> {
            let mut client = client.lock();
            let stmt = format!("SELECT COUNT(*) FROM {qualified_table}");
            let count: i64 = client.query_one(&stmt, &[])?.get(0);
            let count =
                usize::try_from(count).context("PostgreSQL returned a negative memory count")?;
            Ok(count)
        })
        .await?
    }

    async fn health_check(&self) -> bool {
        let client = self.client.clone();
        tokio::task::spawn_blocking(move || client.lock().simple_query("SELECT 1").is_ok())
            .await
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_identifiers_pass_validation() {
        assert!(validate_identifier("public", "schema").is_ok());
        assert!(validate_identifier("_memories_01", "table").is_ok());
    }

    #[test]
    fn invalid_identifiers_are_rejected() {
        assert!(validate_identifier("", "schema").is_err());
        assert!(validate_identifier("1bad", "schema").is_err());
        assert!(validate_identifier("bad-name", "table").is_err());
    }

    #[test]
    fn parse_category_maps_known_and_custom_values() {
        assert_eq!(PostgresMemory::parse_category("core"), MemoryCategory::Core);
        assert_eq!(
            PostgresMemory::parse_category("daily"),
            MemoryCategory::Daily
        );
        assert_eq!(
            PostgresMemory::parse_category("conversation"),
            MemoryCategory::Conversation
        );
        assert_eq!(
            PostgresMemory::parse_category("custom_notes"),
            MemoryCategory::Custom("custom_notes".into())
        );
    }
}
