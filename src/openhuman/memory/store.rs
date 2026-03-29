use super::embeddings::{self, EmbeddingProvider};
use super::traits::{Memory, MemoryCategory, MemoryEntry};
use crate::openhuman::config::{EmbeddingRouteConfig, MemoryConfig, StorageProviderConfig};
use anyhow::Context;
use async_trait::async_trait;
use chrono::Utc;
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

const GLOBAL_NAMESPACE: &str = "global";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceDocumentInput {
    pub namespace: String,
    pub key: String,
    pub title: String,
    pub content: String,
    pub source_type: String,
    pub priority: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub category: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub document_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceQueryResult {
    pub key: String,
    pub content: String,
    pub score: f64,
}

pub struct UnifiedMemory {
    workspace_dir: PathBuf,
    db_path: PathBuf,
    vectors_dir: PathBuf,
    conn: Arc<Mutex<Connection>>,
    embedder: Arc<dyn EmbeddingProvider>,
}

impl UnifiedMemory {
    pub fn new(
        workspace_dir: &Path,
        embedder: Arc<dyn EmbeddingProvider>,
        _open_timeout_secs: Option<u64>,
    ) -> anyhow::Result<Self> {
        let memory_dir = workspace_dir.join("memory");
        let namespaces_dir = memory_dir.join("namespaces");
        let vectors_dir = memory_dir.join("vectors");
        std::fs::create_dir_all(&namespaces_dir)?;
        std::fs::create_dir_all(&vectors_dir)?;

        let db_path = memory_dir.join("memory.db");
        let conn = Connection::open(&db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;

             CREATE TABLE IF NOT EXISTS memory_docs (
               document_id TEXT PRIMARY KEY,
               namespace TEXT NOT NULL,
               key TEXT NOT NULL,
               title TEXT NOT NULL,
               content TEXT NOT NULL,
               source_type TEXT NOT NULL,
               priority TEXT NOT NULL,
               tags_json TEXT NOT NULL,
               metadata_json TEXT NOT NULL,
               category TEXT NOT NULL,
               session_id TEXT,
               created_at REAL NOT NULL,
               updated_at REAL NOT NULL,
               markdown_rel_path TEXT NOT NULL,
               UNIQUE(namespace, key)
             );
             CREATE INDEX IF NOT EXISTS idx_memory_docs_ns_updated ON memory_docs(namespace, updated_at DESC);

             CREATE TABLE IF NOT EXISTS kv_global (
               key TEXT PRIMARY KEY,
               value_json TEXT NOT NULL,
               updated_at REAL NOT NULL
             );

             CREATE TABLE IF NOT EXISTS kv_namespace (
               namespace TEXT NOT NULL,
               key TEXT NOT NULL,
               value_json TEXT NOT NULL,
               updated_at REAL NOT NULL,
               PRIMARY KEY(namespace, key)
             );
             CREATE INDEX IF NOT EXISTS idx_kv_namespace_ns ON kv_namespace(namespace);

             CREATE TABLE IF NOT EXISTS graph_global (
               subject TEXT NOT NULL,
               predicate TEXT NOT NULL,
               object TEXT NOT NULL,
               attrs_json TEXT NOT NULL,
               updated_at REAL NOT NULL,
               PRIMARY KEY(subject, predicate, object)
             );

             CREATE TABLE IF NOT EXISTS graph_namespace (
               namespace TEXT NOT NULL,
               subject TEXT NOT NULL,
               predicate TEXT NOT NULL,
               object TEXT NOT NULL,
               attrs_json TEXT NOT NULL,
               updated_at REAL NOT NULL,
               PRIMARY KEY(namespace, subject, predicate, object)
             );
             CREATE INDEX IF NOT EXISTS idx_graph_namespace_ns ON graph_namespace(namespace);

             CREATE TABLE IF NOT EXISTS vector_chunks (
               namespace TEXT NOT NULL,
               document_id TEXT NOT NULL,
               chunk_id TEXT NOT NULL,
               text TEXT NOT NULL,
               embedding BLOB,
               metadata_json TEXT NOT NULL,
               created_at REAL NOT NULL,
               updated_at REAL NOT NULL,
               PRIMARY KEY(namespace, chunk_id)
             );
             CREATE INDEX IF NOT EXISTS idx_vector_chunks_ns_doc ON vector_chunks(namespace, document_id);",
        )?;

        Ok(Self {
            workspace_dir: workspace_dir.to_path_buf(),
            db_path,
            vectors_dir,
            conn: Arc::new(Mutex::new(conn)),
            embedder,
        })
    }

    pub fn workspace_dir(&self) -> &Path {
        &self.workspace_dir
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn vectors_dir(&self) -> &Path {
        &self.vectors_dir
    }

    fn now_ts() -> f64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    }

    fn sanitize_namespace(namespace: &str) -> String {
        let trimmed = namespace.trim();
        if trimmed.is_empty() {
            return GLOBAL_NAMESPACE.to_string();
        }
        trimmed
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '/' {
                    ch
                } else {
                    '_'
                }
            })
            .collect()
    }

    fn namespace_dir(&self, namespace: &str) -> PathBuf {
        self.workspace_dir
            .join("memory")
            .join("namespaces")
            .join(Self::sanitize_namespace(namespace))
    }

    fn write_markdown_doc(
        &self,
        namespace: &str,
        doc_id: &str,
        title: &str,
        source_type: &str,
        priority: &str,
        tags: &[String],
        created_at: f64,
        updated_at: f64,
        content: &str,
    ) -> anyhow::Result<String> {
        let docs_dir = self.namespace_dir(namespace).join("docs");
        std::fs::create_dir_all(&docs_dir)?;
        let rel_path = format!(
            "memory/namespaces/{}/docs/{doc_id}.md",
            Self::sanitize_namespace(namespace)
        );
        let abs_path = self.workspace_dir.join(&rel_path);

        let header = format!(
            "---\ndoc_id: {doc_id}\nnamespace: {}\ntitle: {}\nsource_type: {}\npriority: {}\ntags: {}\ncreated_at: {}\nupdated_at: {}\n---\n\n",
            namespace.replace('\n', " "),
            title.replace('\n', " "),
            source_type.replace('\n', " "),
            priority.replace('\n', " "),
            serde_json::to_string(tags).unwrap_or_else(|_| "[]".to_string()),
            created_at,
            updated_at
        );
        std::fs::write(abs_path, format!("{header}{content}\n"))?;
        Ok(rel_path)
    }

    fn vec_to_bytes(v: &[f32]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(v.len() * 4);
        for &f in v {
            bytes.extend_from_slice(&f.to_le_bytes());
        }
        bytes
    }

    fn bytes_to_vec(bytes: &[u8]) -> Vec<f32> {
        bytes
            .chunks_exact(4)
            .map(|chunk| {
                let arr: [u8; 4] = chunk.try_into().unwrap_or([0; 4]);
                f32::from_le_bytes(arr)
            })
            .collect()
    }

    fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
        if a.len() != b.len() || a.is_empty() {
            return 0.0;
        }
        let mut dot = 0.0_f64;
        let mut norm_a = 0.0_f64;
        let mut norm_b = 0.0_f64;
        for (x, y) in a.iter().zip(b.iter()) {
            let x = f64::from(*x);
            let y = f64::from(*y);
            dot += x * y;
            norm_a += x * x;
            norm_b += y * y;
        }
        let denom = norm_a.sqrt() * norm_b.sqrt();
        if denom <= f64::EPSILON {
            return 0.0;
        }
        (dot / denom).clamp(0.0, 1.0)
    }

    fn split_chunks(content: &str, max_len: usize) -> Vec<String> {
        let mut out = Vec::new();
        let mut current = String::new();
        for para in content.split("\n\n") {
            let p = para.trim();
            if p.is_empty() {
                continue;
            }
            if current.is_empty() {
                current.push_str(p);
                continue;
            }
            if current.len() + 2 + p.len() <= max_len {
                current.push_str("\n\n");
                current.push_str(p);
            } else {
                out.push(std::mem::take(&mut current));
                current.push_str(p);
            }
        }
        if !current.trim().is_empty() {
            out.push(current);
        }
        out
    }

    pub async fn upsert_document(&self, input: NamespaceDocumentInput) -> Result<String, String> {
        let namespace = Self::sanitize_namespace(&input.namespace);
        let key = input.key.trim().to_string();
        if key.is_empty() {
            return Err("document key cannot be empty".to_string());
        }
        let document_id = input
            .document_id
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let now = Self::now_ts();
        let created_at = now;
        let updated_at = now;
        let markdown_rel = self
            .write_markdown_doc(
                &namespace,
                &document_id,
                &input.title,
                &input.source_type,
                &input.priority,
                &input.tags,
                created_at,
                updated_at,
                &input.content,
            )
            .map_err(|e| e.to_string())?;

        let tags_json = serde_json::to_string(&input.tags).map_err(|e| e.to_string())?;
        let metadata_json = input.metadata.to_string();

        {
            let conn = self.conn.lock();
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("begin tx: {e}"))?;
            tx.execute(
                "INSERT INTO memory_docs
                  (document_id, namespace, key, title, content, source_type, priority, tags_json, metadata_json, category, session_id, created_at, updated_at, markdown_rel_path)
                 VALUES
                  (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                 ON CONFLICT(namespace, key) DO UPDATE SET
                  title = excluded.title,
                  content = excluded.content,
                  source_type = excluded.source_type,
                  priority = excluded.priority,
                  tags_json = excluded.tags_json,
                  metadata_json = excluded.metadata_json,
                  category = excluded.category,
                  session_id = excluded.session_id,
                  updated_at = excluded.updated_at,
                  markdown_rel_path = excluded.markdown_rel_path",
                params![
                    document_id,
                    namespace,
                    key,
                    input.title,
                    input.content,
                    input.source_type,
                    input.priority,
                    tags_json,
                    metadata_json,
                    input.category,
                    input.session_id,
                    created_at,
                    updated_at,
                    markdown_rel
                ],
            )
            .map_err(|e| format!("upsert memory_docs: {e}"))?;
            tx.execute(
                "DELETE FROM vector_chunks WHERE namespace = ?1 AND document_id = ?2",
                params![namespace, document_id],
            )
            .map_err(|e| format!("clear vector chunks: {e}"))?;
            tx.commit().map_err(|e| format!("commit tx: {e}"))?;
        }

        let chunks = Self::split_chunks(&input.content, 900);
        for (idx, chunk) in chunks.iter().enumerate() {
            let embedding = self
                .embedder
                .embed_one(chunk)
                .await
                .ok()
                .map(|v| Self::vec_to_bytes(&v));
            let chunk_id = format!("{document_id}:{idx}");
            let conn = self.conn.lock();
            conn.execute(
                "INSERT OR REPLACE INTO vector_chunks
                  (namespace, document_id, chunk_id, text, embedding, metadata_json, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    namespace,
                    document_id,
                    chunk_id,
                    chunk,
                    embedding,
                    json!({"lancedb_table": format!("ns_{}", namespace), "chunk_index": idx}).to_string(),
                    now,
                    now
                ],
            )
            .map_err(|e| format!("insert vector chunk: {e}"))?;
        }

        Ok(document_id)
    }

    pub async fn list_documents(
        &self,
        namespace: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let conn = self.conn.lock();
        let mut docs = Vec::new();
        if let Some(ns) = namespace {
            let mut stmt = conn
                .prepare(
                    "SELECT document_id, namespace, key, title, source_type, priority, created_at, updated_at
                     FROM memory_docs WHERE namespace = ?1 ORDER BY updated_at DESC",
                )
                .map_err(|e| format!("prepare list_documents: {e}"))?;
            let mut rows = stmt
                .query(params![Self::sanitize_namespace(ns)])
                .map_err(|e| format!("query list_documents: {e}"))?;
            while let Some(row) = rows
                .next()
                .map_err(|e| format!("row list_documents: {e}"))?
            {
                docs.push(json!({
                    "documentId": row.get::<_, String>(0).map_err(|e| e.to_string())?,
                    "namespace": row.get::<_, String>(1).map_err(|e| e.to_string())?,
                    "key": row.get::<_, String>(2).map_err(|e| e.to_string())?,
                    "title": row.get::<_, String>(3).map_err(|e| e.to_string())?,
                    "sourceType": row.get::<_, String>(4).map_err(|e| e.to_string())?,
                    "priority": row.get::<_, String>(5).map_err(|e| e.to_string())?,
                    "createdAt": row.get::<_, f64>(6).map_err(|e| e.to_string())?,
                    "updatedAt": row.get::<_, f64>(7).map_err(|e| e.to_string())?,
                }));
            }
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT document_id, namespace, key, title, source_type, priority, created_at, updated_at
                     FROM memory_docs ORDER BY updated_at DESC",
                )
                .map_err(|e| format!("prepare list_documents: {e}"))?;
            let mut rows = stmt
                .query([])
                .map_err(|e| format!("query list_documents: {e}"))?;
            while let Some(row) = rows
                .next()
                .map_err(|e| format!("row list_documents: {e}"))?
            {
                docs.push(json!({
                    "documentId": row.get::<_, String>(0).map_err(|e| e.to_string())?,
                    "namespace": row.get::<_, String>(1).map_err(|e| e.to_string())?,
                    "key": row.get::<_, String>(2).map_err(|e| e.to_string())?,
                    "title": row.get::<_, String>(3).map_err(|e| e.to_string())?,
                    "sourceType": row.get::<_, String>(4).map_err(|e| e.to_string())?,
                    "priority": row.get::<_, String>(5).map_err(|e| e.to_string())?,
                    "createdAt": row.get::<_, f64>(6).map_err(|e| e.to_string())?,
                    "updatedAt": row.get::<_, f64>(7).map_err(|e| e.to_string())?,
                }));
            }
        }
        Ok(json!({ "documents": docs, "count": docs.len() }))
    }

    pub async fn list_namespaces(&self) -> Result<Vec<String>, String> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT DISTINCT namespace FROM memory_docs ORDER BY namespace")
            .map_err(|e| format!("prepare list_namespaces: {e}"))?;
        let mut rows = stmt
            .query([])
            .map_err(|e| format!("query list_namespaces: {e}"))?;
        let mut out = BTreeSet::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| format!("row list_namespaces: {e}"))?
        {
            let ns: String = row.get(0).map_err(|e| e.to_string())?;
            if !ns.trim().is_empty() {
                out.insert(ns);
            }
        }
        Ok(out.into_iter().collect())
    }

    pub async fn delete_document(
        &self,
        namespace: &str,
        document_id: &str,
    ) -> Result<serde_json::Value, String> {
        let ns = Self::sanitize_namespace(namespace);
        let conn = self.conn.lock();
        let rel_path: Option<String> = conn
            .query_row(
                "SELECT markdown_rel_path FROM memory_docs WHERE namespace = ?1 AND document_id = ?2",
                params![ns, document_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("query delete_document path: {e}"))?;
        let deleted = conn
            .execute(
                "DELETE FROM memory_docs WHERE namespace = ?1 AND document_id = ?2",
                params![ns, document_id],
            )
            .map_err(|e| format!("delete memory_doc: {e}"))?
            > 0;
        conn.execute(
            "DELETE FROM vector_chunks WHERE namespace = ?1 AND document_id = ?2",
            params![ns, document_id],
        )
        .map_err(|e| format!("delete vector_chunks: {e}"))?;

        if let Some(rel) = rel_path {
            let abs = self.workspace_dir.join(rel);
            let _ = std::fs::remove_file(abs);
        }
        Ok(json!({"deleted": deleted, "namespace": ns, "documentId": document_id }))
    }

    pub async fn query_namespace_context(
        &self,
        namespace: &str,
        query: &str,
        limit: u32,
    ) -> Result<String, String> {
        let ns = Self::sanitize_namespace(namespace);
        let query_terms: Vec<String> = query
            .split_whitespace()
            .map(|t| t.trim().to_ascii_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        let (keyword_scores, by_key) = {
            let conn = self.conn.lock();
            let mut stmt = conn
                .prepare(
                    "SELECT key, content, updated_at FROM memory_docs
                     WHERE namespace = ?1 ORDER BY updated_at DESC LIMIT 400",
                )
                .map_err(|e| format!("prepare query_namespace_context: {e}"))?;
            let mut rows = stmt
                .query(params![ns])
                .map_err(|e| format!("query query_namespace_context: {e}"))?;
            let mut keyword_scores: HashMap<String, f64> = HashMap::new();
            let mut by_key: HashMap<String, String> = HashMap::new();
            while let Some(row) = rows
                .next()
                .map_err(|e| format!("row query_namespace_context: {e}"))?
            {
                let key: String = row.get(0).map_err(|e| e.to_string())?;
                let content: String = row.get(1).map_err(|e| e.to_string())?;
                let lower = format!("{key} {content}").to_ascii_lowercase();
                let matched = if query_terms.is_empty() {
                    1
                } else {
                    query_terms
                        .iter()
                        .filter(|term| lower.contains(term.as_str()))
                        .count()
                };
                if matched > 0 {
                    let score = matched as f64 / (query_terms.len().max(1) as f64);
                    keyword_scores.insert(key.clone(), score);
                    by_key.insert(key, content);
                }
            }
            (keyword_scores, by_key)
        };

        let vector_scores = self
            .query_vector_scores(&ns, query)
            .await
            .unwrap_or_default();
        let mut merged: Vec<NamespaceQueryResult> = by_key
            .into_iter()
            .map(|(key, content)| {
                let k = keyword_scores.get(&key).copied().unwrap_or(0.0);
                let v = vector_scores.get(&key).copied().unwrap_or(0.0);
                NamespaceQueryResult {
                    key,
                    content,
                    score: 0.7 * v + 0.3 * k,
                }
            })
            .collect();
        merged.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        merged.truncate(limit as usize);
        Ok(merged
            .into_iter()
            .map(|r| format!("{}: {}", r.key, r.content))
            .collect::<Vec<_>>()
            .join("\n\n"))
    }

    async fn query_vector_scores(
        &self,
        namespace: &str,
        query: &str,
    ) -> Result<HashMap<String, f64>, String> {
        let embedding = self
            .embedder
            .embed_one(query)
            .await
            .map_err(|e| format!("embedding query: {e}"))?;
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT vc.embedding, md.key
                 FROM vector_chunks vc
                 JOIN memory_docs md ON md.document_id = vc.document_id AND md.namespace = vc.namespace
                 WHERE vc.namespace = ?1 AND vc.embedding IS NOT NULL",
            )
            .map_err(|e| format!("prepare query_vector_scores: {e}"))?;
        let mut rows = stmt
            .query(params![namespace])
            .map_err(|e| format!("query query_vector_scores: {e}"))?;
        let mut per_key = HashMap::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| format!("row query_vector_scores: {e}"))?
        {
            let blob: Vec<u8> = row.get(0).map_err(|e| e.to_string())?;
            let key: String = row.get(1).map_err(|e| e.to_string())?;
            let chunk_vec = Self::bytes_to_vec(&blob);
            let sim = Self::cosine_similarity(&embedding, &chunk_vec);
            let old = per_key.get(&key).copied().unwrap_or(0.0);
            if sim > old {
                per_key.insert(key, sim);
            }
        }
        Ok(per_key)
    }

    pub async fn recall_namespace_context(
        &self,
        namespace: &str,
        max_chunks: u32,
    ) -> Result<Option<String>, String> {
        let ns = Self::sanitize_namespace(namespace);
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT content FROM memory_docs
                 WHERE namespace = ?1
                 ORDER BY updated_at DESC
                 LIMIT ?2",
            )
            .map_err(|e| format!("prepare recall_namespace_context: {e}"))?;
        let mut rows = stmt
            .query(params![ns, i64::from(max_chunks)])
            .map_err(|e| format!("query recall_namespace_context: {e}"))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| format!("row recall_namespace_context: {e}"))?
        {
            out.push(row.get::<_, String>(0).map_err(|e| e.to_string())?);
        }
        if out.is_empty() {
            Ok(None)
        } else {
            Ok(Some(out.join("\n\n")))
        }
    }

    pub async fn kv_set_global(&self, key: &str, value: &serde_json::Value) -> Result<(), String> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO kv_global (key, value_json, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
            params![key, value.to_string(), Self::now_ts()],
        )
        .map_err(|e| format!("kv_set_global: {e}"))?;
        Ok(())
    }

    pub async fn kv_get_global(&self, key: &str) -> Result<Option<serde_json::Value>, String> {
        let conn = self.conn.lock();
        let value: Option<String> = conn
            .query_row(
                "SELECT value_json FROM kv_global WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("kv_get_global: {e}"))?;
        Ok(value.and_then(|v| serde_json::from_str(&v).ok()))
    }

    pub async fn kv_set_namespace(
        &self,
        namespace: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), String> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO kv_namespace (namespace, key, value_json, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(namespace, key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
            params![Self::sanitize_namespace(namespace), key, value.to_string(), Self::now_ts()],
        )
        .map_err(|e| format!("kv_set_namespace: {e}"))?;
        Ok(())
    }

    pub async fn kv_get_namespace(
        &self,
        namespace: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, String> {
        let conn = self.conn.lock();
        let value: Option<String> = conn
            .query_row(
                "SELECT value_json FROM kv_namespace WHERE namespace = ?1 AND key = ?2",
                params![Self::sanitize_namespace(namespace), key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("kv_get_namespace: {e}"))?;
        Ok(value.and_then(|v| serde_json::from_str(&v).ok()))
    }

    pub async fn kv_delete_global(&self, key: &str) -> Result<bool, String> {
        let conn = self.conn.lock();
        let changed = conn
            .execute("DELETE FROM kv_global WHERE key = ?1", params![key])
            .map_err(|e| format!("kv_delete_global: {e}"))?;
        Ok(changed > 0)
    }

    pub async fn kv_delete_namespace(&self, namespace: &str, key: &str) -> Result<bool, String> {
        let conn = self.conn.lock();
        let changed = conn
            .execute(
                "DELETE FROM kv_namespace WHERE namespace = ?1 AND key = ?2",
                params![Self::sanitize_namespace(namespace), key],
            )
            .map_err(|e| format!("kv_delete_namespace: {e}"))?;
        Ok(changed > 0)
    }

    pub async fn kv_list_namespace(
        &self,
        namespace: &str,
    ) -> Result<Vec<serde_json::Value>, String> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT key, value_json, updated_at FROM kv_namespace
                 WHERE namespace = ?1 ORDER BY updated_at DESC",
            )
            .map_err(|e| format!("kv_list_namespace prepare: {e}"))?;
        let mut rows = stmt
            .query(params![Self::sanitize_namespace(namespace)])
            .map_err(|e| format!("kv_list_namespace query: {e}"))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| format!("kv_list_namespace row: {e}"))?
        {
            let value_raw: String = row.get(1).map_err(|e| e.to_string())?;
            out.push(json!({
                "key": row.get::<_, String>(0).map_err(|e| e.to_string())?,
                "value": serde_json::from_str::<serde_json::Value>(&value_raw).unwrap_or(serde_json::Value::Null),
                "updatedAt": row.get::<_, f64>(2).map_err(|e| e.to_string())?,
            }));
        }
        Ok(out)
    }

    pub async fn graph_upsert_global(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        attrs: &serde_json::Value,
    ) -> Result<(), String> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO graph_global (subject, predicate, object, attrs_json, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(subject, predicate, object) DO UPDATE SET attrs_json = excluded.attrs_json, updated_at = excluded.updated_at",
            params![subject, predicate, object, attrs.to_string(), Self::now_ts()],
        )
        .map_err(|e| format!("graph_upsert_global: {e}"))?;
        Ok(())
    }

    pub async fn graph_upsert_namespace(
        &self,
        namespace: &str,
        subject: &str,
        predicate: &str,
        object: &str,
        attrs: &serde_json::Value,
    ) -> Result<(), String> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO graph_namespace (namespace, subject, predicate, object, attrs_json, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(namespace, subject, predicate, object) DO UPDATE SET attrs_json = excluded.attrs_json, updated_at = excluded.updated_at",
            params![
                Self::sanitize_namespace(namespace),
                subject,
                predicate,
                object,
                attrs.to_string(),
                Self::now_ts()
            ],
        )
        .map_err(|e| format!("graph_upsert_namespace: {e}"))?;
        Ok(())
    }

    pub async fn graph_query_global(
        &self,
        subject: Option<&str>,
        predicate: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, String> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT subject, predicate, object, attrs_json, updated_at
                 FROM graph_global
                 WHERE (?1 IS NULL OR subject = ?1)
                   AND (?2 IS NULL OR predicate = ?2)
                 ORDER BY updated_at DESC
                 LIMIT 300",
            )
            .map_err(|e| format!("graph_query_global prepare: {e}"))?;
        let mut rows = stmt
            .query(params![subject, predicate])
            .map_err(|e| format!("graph_query_global query: {e}"))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| format!("graph_query_global row: {e}"))?
        {
            let attrs_raw: String = row.get(3).map_err(|e| e.to_string())?;
            out.push(json!({
                "subject": row.get::<_, String>(0).map_err(|e| e.to_string())?,
                "predicate": row.get::<_, String>(1).map_err(|e| e.to_string())?,
                "object": row.get::<_, String>(2).map_err(|e| e.to_string())?,
                "attrs": serde_json::from_str::<serde_json::Value>(&attrs_raw).unwrap_or_else(|_| json!({})),
                "updatedAt": row.get::<_, f64>(4).map_err(|e| e.to_string())?,
            }));
        }
        Ok(out)
    }

    pub async fn graph_query_namespace(
        &self,
        namespace: &str,
        subject: Option<&str>,
        predicate: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, String> {
        let conn = self.conn.lock();
        let ns = Self::sanitize_namespace(namespace);
        let mut stmt = conn
            .prepare(
                "SELECT subject, predicate, object, attrs_json, updated_at
                 FROM graph_namespace
                 WHERE namespace = ?1
                 AND (?2 IS NULL OR subject = ?2)
                 AND (?3 IS NULL OR predicate = ?3)
                 ORDER BY updated_at DESC
                 LIMIT 300",
            )
            .map_err(|e| format!("graph_query_namespace prepare: {e}"))?;
        let mut rows = stmt
            .query(params![ns, subject, predicate])
            .map_err(|e| format!("graph_query_namespace query: {e}"))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| format!("graph_query_namespace row: {e}"))?
        {
            let attrs_raw: String = row.get(3).map_err(|e| e.to_string())?;
            out.push(json!({
                "subject": row.get::<_, String>(0).map_err(|e| e.to_string())?,
                "predicate": row.get::<_, String>(1).map_err(|e| e.to_string())?,
                "object": row.get::<_, String>(2).map_err(|e| e.to_string())?,
                "attrs": serde_json::from_str::<serde_json::Value>(&attrs_raw).unwrap_or_else(|_| json!({})),
                "updatedAt": row.get::<_, f64>(4).map_err(|e| e.to_string())?,
            }));
        }
        Ok(out)
    }
}

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

pub type MemoryClientRef = Arc<MemoryClient>;

pub struct MemoryState(pub std::sync::Mutex<Option<MemoryClientRef>>);

#[derive(Clone)]
pub struct MemoryClient {
    inner: Arc<UnifiedMemory>,
}

impl MemoryClient {
    pub fn from_token(_jwt_token: String) -> Option<Self> {
        Self::new_local().ok()
    }

    pub fn new_local() -> Result<Self, String> {
        let workspace_dir = dirs::home_dir()
            .ok_or_else(|| "Failed to resolve home directory".to_string())?
            .join(".openhuman")
            .join("workspace");
        std::fs::create_dir_all(&workspace_dir)
            .map_err(|e| format!("Create workspace dir {}: {e}", workspace_dir.display()))?;
        let embedder = Arc::new(embeddings::NoopEmbedding);
        let memory =
            UnifiedMemory::new(&workspace_dir, embedder, None).map_err(|e| format!("{e}"))?;
        Ok(Self {
            inner: Arc::new(memory),
        })
    }

    pub async fn put_doc(&self, input: NamespaceDocumentInput) -> Result<String, String> {
        self.inner.upsert_document(input).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn store_skill_sync(
        &self,
        skill_id: &str,
        _integration_id: &str,
        title: &str,
        content: &str,
        source_type: Option<String>,
        metadata: Option<serde_json::Value>,
        priority: Option<String>,
        _created_at: Option<f64>,
        _updated_at: Option<f64>,
        document_id: Option<String>,
    ) -> Result<(), String> {
        let namespace = format!("skill-{}", skill_id.trim());
        self.inner
            .upsert_document(NamespaceDocumentInput {
                namespace,
                key: title.to_string(),
                title: title.to_string(),
                content: content.to_string(),
                source_type: source_type.unwrap_or_else(|| "doc".to_string()),
                priority: priority.unwrap_or_else(|| "medium".to_string()),
                tags: Vec::new(),
                metadata: metadata.unwrap_or_else(|| json!({})),
                category: "core".to_string(),
                session_id: None,
                document_id,
            })
            .await
            .map(|_| ())
    }

    pub async fn list_documents(
        &self,
        namespace: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        self.inner.list_documents(namespace).await
    }

    pub async fn list_namespaces(&self) -> Result<Vec<String>, String> {
        self.inner.list_namespaces().await
    }

    pub async fn delete_document(
        &self,
        namespace: &str,
        document_id: &str,
    ) -> Result<serde_json::Value, String> {
        self.inner.delete_document(namespace, document_id).await
    }

    pub async fn clear_skill_memory(
        &self,
        skill_id: &str,
        _integration_id: &str,
    ) -> Result<(), String> {
        let namespace = format!("skill-{}", skill_id.trim());
        let docs = self.list_documents(Some(&namespace)).await?;
        let items = docs
            .get("documents")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        for item in items {
            if let Some(document_id) = item.get("documentId").and_then(serde_json::Value::as_str) {
                let _ = self.delete_document(&namespace, document_id).await?;
            }
        }
        Ok(())
    }

    pub async fn query_namespace(
        &self,
        namespace: &str,
        query: &str,
        max_chunks: u32,
    ) -> Result<String, String> {
        self.inner
            .query_namespace_context(namespace, query, max_chunks)
            .await
    }

    pub async fn recall_namespace(
        &self,
        namespace: &str,
        max_chunks: u32,
    ) -> Result<Option<String>, String> {
        self.inner
            .recall_namespace_context(namespace, max_chunks)
            .await
    }

    pub async fn kv_set(
        &self,
        namespace: Option<&str>,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), String> {
        match namespace {
            Some(ns) => self.inner.kv_set_namespace(ns, key, value).await,
            None => self.inner.kv_set_global(key, value).await,
        }
    }

    pub async fn kv_get(
        &self,
        namespace: Option<&str>,
        key: &str,
    ) -> Result<Option<serde_json::Value>, String> {
        match namespace {
            Some(ns) => self.inner.kv_get_namespace(ns, key).await,
            None => self.inner.kv_get_global(key).await,
        }
    }

    pub async fn kv_delete(&self, namespace: Option<&str>, key: &str) -> Result<bool, String> {
        match namespace {
            Some(ns) => self.inner.kv_delete_namespace(ns, key).await,
            None => self.inner.kv_delete_global(key).await,
        }
    }

    pub async fn kv_list_namespace(
        &self,
        namespace: &str,
    ) -> Result<Vec<serde_json::Value>, String> {
        self.inner.kv_list_namespace(namespace).await
    }

    pub async fn graph_upsert(
        &self,
        namespace: Option<&str>,
        subject: &str,
        predicate: &str,
        object: &str,
        attrs: &serde_json::Value,
    ) -> Result<(), String> {
        match namespace {
            Some(ns) => {
                self.inner
                    .graph_upsert_namespace(ns, subject, predicate, object, attrs)
                    .await
            }
            None => {
                self.inner
                    .graph_upsert_global(subject, predicate, object, attrs)
                    .await
            }
        }
    }

    pub async fn graph_query(
        &self,
        namespace: Option<&str>,
        subject: Option<&str>,
        predicate: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, String> {
        match namespace {
            Some(ns) => {
                self.inner
                    .graph_query_namespace(ns, subject, predicate)
                    .await
            }
            None => self.inner.graph_query_global(subject, predicate).await,
        }
    }
}

pub fn effective_memory_backend_name(
    _memory_backend: &str,
    _storage_provider: Option<&StorageProviderConfig>,
) -> String {
    "namespace".to_string()
}

pub fn create_memory(
    config: &MemoryConfig,
    workspace_dir: &Path,
    api_key: Option<&str>,
) -> anyhow::Result<Box<dyn Memory>> {
    create_memory_with_storage_and_routes(config, &[], None, workspace_dir, api_key)
}

pub fn create_memory_with_storage(
    config: &MemoryConfig,
    storage_provider: Option<&StorageProviderConfig>,
    workspace_dir: &Path,
    api_key: Option<&str>,
) -> anyhow::Result<Box<dyn Memory>> {
    create_memory_with_storage_and_routes(config, &[], storage_provider, workspace_dir, api_key)
}

pub fn create_memory_with_storage_and_routes(
    config: &MemoryConfig,
    _embedding_routes: &[EmbeddingRouteConfig],
    _storage_provider: Option<&StorageProviderConfig>,
    workspace_dir: &Path,
    api_key: Option<&str>,
) -> anyhow::Result<Box<dyn Memory>> {
    let embedder: Arc<dyn EmbeddingProvider> = Arc::from(embeddings::create_embedding_provider(
        &config.embedding_provider,
        api_key,
        &config.embedding_model,
        config.embedding_dimensions,
    ));
    let mem = UnifiedMemory::new(workspace_dir, embedder, config.sqlite_open_timeout_secs)?;
    Ok(Box::new(mem))
}

pub fn create_memory_for_migration(
    _backend: &str,
    _workspace_dir: &Path,
) -> anyhow::Result<Box<dyn Memory>> {
    anyhow::bail!("memory migration is disabled for the unified namespace memory core")
}
