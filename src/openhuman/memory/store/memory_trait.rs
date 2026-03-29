use async_trait::async_trait;
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde_json::json;

use crate::openhuman::memory::store::types::{NamespaceDocumentInput, GLOBAL_NAMESPACE};
use crate::openhuman::memory::traits::{Memory, MemoryCategory, MemoryEntry};
use anyhow::Context;

use super::unified::UnifiedMemory;

#[async_trait]
impl Memory for UnifiedMemory {
    fn name(&self) -> &str {
        "namespace"
    }

    async fn store(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        self.upsert_document(NamespaceDocumentInput {
            namespace: GLOBAL_NAMESPACE.to_string(),
            key: key.to_string(),
            title: key.to_string(),
            content: content.to_string(),
            source_type: "chat".to_string(),
            priority: "medium".to_string(),
            tags: Vec::new(),
            metadata: json!({}),
            category: category.to_string(),
            session_id: session_id.map(str::to_string),
            document_id: None,
        })
        .await
        .map(|_| ())
        .map_err(anyhow::Error::msg)
    }

    async fn recall(
        &self,
        query: &str,
        limit: usize,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        let text = self
            .query_namespace_context(GLOBAL_NAMESPACE, query, limit as u32)
            .await
            .map_err(anyhow::Error::msg)?;
        let out = text
            .lines()
            .enumerate()
            .filter_map(|(idx, line)| {
                let (key, content) = line.split_once(": ")?;
                Some(MemoryEntry {
                    id: format!("global:{idx}"),
                    key: key.to_string(),
                    content: content.to_string(),
                    namespace: Some(GLOBAL_NAMESPACE.to_string()),
                    category: MemoryCategory::Core,
                    timestamp: Utc::now().to_rfc3339(),
                    session_id: None,
                    score: Some(1.0),
                })
            })
            .collect();
        Ok(out)
    }

    async fn get(&self, key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        let conn = self.conn.lock();
        let row: Option<(String, String, String, f64)> = conn
            .query_row(
                "SELECT document_id, key, content, updated_at
                 FROM memory_docs WHERE namespace = ?1 AND key = ?2 LIMIT 1",
                params![GLOBAL_NAMESPACE, key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        Ok(row.map(|(id, key, content, updated_at)| MemoryEntry {
            id,
            key,
            content,
            namespace: Some(GLOBAL_NAMESPACE.to_string()),
            category: MemoryCategory::Core,
            timestamp: format!("{updated_at}"),
            session_id: None,
            score: None,
        }))
    }

    async fn list(
        &self,
        category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        let docs = self
            .list_documents(Some(GLOBAL_NAMESPACE))
            .await
            .map_err(anyhow::Error::msg)?;
        let mut out = Vec::new();
        let items = docs
            .get("documents")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        for (idx, d) in items.into_iter().enumerate() {
            let cat = category.cloned().unwrap_or(MemoryCategory::Core);
            out.push(MemoryEntry {
                id: d
                    .get("documentId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                key: d
                    .get("key")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                content: d
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                namespace: Some(GLOBAL_NAMESPACE.to_string()),
                category: cat,
                timestamp: format!("idx-{idx}"),
                session_id: None,
                score: None,
            });
        }
        Ok(out)
    }

    async fn forget(&self, key: &str) -> anyhow::Result<bool> {
        let row: Option<String> = {
            let conn = self.conn.lock();
            conn.query_row(
                "SELECT document_id FROM memory_docs WHERE namespace = ?1 AND key = ?2 LIMIT 1",
                params![GLOBAL_NAMESPACE, key],
                |row| row.get(0),
            )
            .optional()?
        };
        let Some(document_id) = row else {
            return Ok(false);
        };
        self.delete_document(GLOBAL_NAMESPACE, &document_id)
            .await
            .map_err(anyhow::Error::msg)?;
        Ok(true)
    }

    async fn count(&self) -> anyhow::Result<usize> {
        let conn = self.conn.lock();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_docs WHERE namespace = ?1",
            params![GLOBAL_NAMESPACE],
            |row| row.get(0),
        )?;
        usize::try_from(count).context("negative count")
    }

    async fn health_check(&self) -> bool {
        self.workspace_dir.exists() && self.db_path.exists()
    }
}
