//! Knowledge-graph relations stored in `graph_namespace` and `graph_global`.
//!
//! Provides upsert (with attribute merging + evidence accumulation), namespace
//! / global / cross-namespace queries, and the document-scoped removal used
//! when a source document is deleted or re-ingested.

use rusqlite::{params, OptionalExtension};
use serde_json::{json, Map, Value};

use crate::openhuman::memory_store::types::GraphRelationRecord;

use super::UnifiedMemory;

impl UnifiedMemory {
    pub(crate) async fn graph_remove_document_namespace(
        &self,
        namespace: &str,
        document_id: &str,
    ) -> Result<(), String> {
        let relations = self
            .graph_relations_namespace(namespace, None, None)
            .await?;
        if relations.is_empty() {
            return Ok(());
        }

        let doc_prefix = format!("{document_id}:");
        let updated_at = Self::now_ts();
        let conn = self.conn.lock();
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("graph_remove_document_namespace begin tx: {e}"))?;

        for relation in relations {
            let touches_document = relation.document_ids.iter().any(|id| id == document_id)
                || relation
                    .chunk_ids
                    .iter()
                    .any(|chunk_id| chunk_id.starts_with(&doc_prefix));
            if !touches_document {
                continue;
            }

            let mut attrs = relation.attrs.as_object().cloned().unwrap_or_default();
            let document_ids = relation
                .document_ids
                .iter()
                .filter(|id| id.as_str() != document_id)
                .cloned()
                .collect::<Vec<_>>();
            let chunk_ids = relation
                .chunk_ids
                .iter()
                .filter(|chunk_id| !chunk_id.starts_with(&doc_prefix))
                .cloned()
                .collect::<Vec<_>>();

            if document_ids.is_empty() && chunk_ids.is_empty() {
                tx.execute(
                    "DELETE FROM graph_namespace
                     WHERE namespace = ?1 AND subject = ?2 AND predicate = ?3 AND object = ?4",
                    params![
                        Self::sanitize_namespace(namespace),
                        relation.subject,
                        relation.predicate,
                        relation.object
                    ],
                )
                .map_err(|e| format!("graph_remove_document_namespace delete: {e}"))?;
                continue;
            }

            attrs.insert("document_ids".to_string(), json!(document_ids));
            if chunk_ids.is_empty() {
                attrs.remove("chunk_ids");
            } else {
                attrs.insert("chunk_ids".to_string(), json!(chunk_ids.clone()));
            }
            attrs.insert("evidence_count".to_string(), json!(chunk_ids.len().max(1)));
            attrs.insert("updated_at".to_string(), json!(updated_at));

            tx.execute(
                "UPDATE graph_namespace
                 SET attrs_json = ?1, updated_at = ?2
                 WHERE namespace = ?3 AND subject = ?4 AND predicate = ?5 AND object = ?6",
                params![
                    Value::Object(attrs).to_string(),
                    updated_at,
                    Self::sanitize_namespace(namespace),
                    relation.subject,
                    relation.predicate,
                    relation.object
                ],
            )
            .map_err(|e| format!("graph_remove_document_namespace update: {e}"))?;
        }

        tx.commit()
            .map_err(|e| format!("graph_remove_document_namespace commit: {e}"))?;
        Ok(())
    }

    /// Upsert a relation into the cross-namespace `graph_global` table.
    pub async fn graph_upsert_global(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        attrs: &serde_json::Value,
    ) -> Result<(), String> {
        self.graph_upsert_internal(None, subject, predicate, object, attrs)
            .await
    }

    /// Upsert a relation into the namespace-scoped `graph_namespace` table,
    /// merging attributes (evidence count, document/chunk ids) with any
    /// existing edge.
    pub async fn graph_upsert_namespace(
        &self,
        namespace: &str,
        subject: &str,
        predicate: &str,
        object: &str,
        attrs: &serde_json::Value,
    ) -> Result<(), String> {
        self.graph_upsert_internal(Some(namespace), subject, predicate, object, attrs)
            .await
    }

    /// Query relations in the global graph with optional subject/predicate filters.
    pub async fn graph_query_global(
        &self,
        subject: Option<&str>,
        predicate: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, String> {
        let rows = self.graph_relations_global(subject, predicate).await?;
        Ok(rows
            .into_iter()
            .map(Self::graph_relation_to_json)
            .collect::<Vec<_>>())
    }

    /// Query all graph relations across every namespace AND global, with
    /// optional subject/predicate filters.  Used when the caller passes no
    /// namespace so that ingested (namespace-scoped) data is still surfaced.
    pub async fn graph_query_all(
        &self,
        subject: Option<&str>,
        predicate: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, String> {
        let mut rows = self
            .graph_relations_all_namespaces(subject, predicate)
            .await?;
        rows.extend(self.graph_relations_global(subject, predicate).await?);
        rows.sort_by(|a, b| {
            b.updated_at
                .partial_cmp(&a.updated_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        rows.truncate(300);
        Ok(rows
            .into_iter()
            .map(Self::graph_relation_to_json)
            .collect::<Vec<_>>())
    }

    /// Query relations within a single namespace with optional subject/predicate filters.
    pub async fn graph_query_namespace(
        &self,
        namespace: &str,
        subject: Option<&str>,
        predicate: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, String> {
        let rows = self
            .graph_relations_namespace(namespace, subject, predicate)
            .await?;
        Ok(rows
            .into_iter()
            .map(Self::graph_relation_to_json)
            .collect::<Vec<_>>())
    }

    pub(crate) async fn graph_relations_for_scope(
        &self,
        namespace: &str,
    ) -> Result<Vec<GraphRelationRecord>, String> {
        let mut rows = self
            .graph_relations_namespace(namespace, None, None)
            .await?;
        rows.extend(self.graph_relations_global(None, None).await?);
        rows.sort_by(|a, b| {
            b.updated_at
                .partial_cmp(&a.updated_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(rows)
    }

    pub(crate) async fn graph_relations_namespace(
        &self,
        namespace: &str,
        subject: Option<&str>,
        predicate: Option<&str>,
    ) -> Result<Vec<GraphRelationRecord>, String> {
        let conn = self.conn.lock();
        let ns = Self::sanitize_namespace(namespace);
        let subject = subject.map(Self::normalize_graph_entity);
        let predicate = predicate.map(Self::normalize_graph_predicate);
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
            .map_err(|e| format!("graph_relations_namespace prepare: {e}"))?;
        let mut rows = stmt
            .query(params![ns, subject, predicate])
            .map_err(|e| format!("graph_relations_namespace query: {e}"))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| format!("graph_relations_namespace row: {e}"))?
        {
            let attrs_raw: String = row.get(3).map_err(|e| e.to_string())?;
            out.push(Self::graph_relation_from_parts(
                Some(Self::sanitize_namespace(namespace)),
                row.get(0).map_err(|e| e.to_string())?,
                row.get(1).map_err(|e| e.to_string())?,
                row.get(2).map_err(|e| e.to_string())?,
                &attrs_raw,
                row.get(4).map_err(|e| e.to_string())?,
            ));
        }
        Ok(out)
    }

    pub(crate) async fn graph_relations_global(
        &self,
        subject: Option<&str>,
        predicate: Option<&str>,
    ) -> Result<Vec<GraphRelationRecord>, String> {
        let conn = self.conn.lock();
        let subject = subject.map(Self::normalize_graph_entity);
        let predicate = predicate.map(Self::normalize_graph_predicate);
        let mut stmt = conn
            .prepare(
                "SELECT subject, predicate, object, attrs_json, updated_at
                 FROM graph_global
                 WHERE (?1 IS NULL OR subject = ?1)
                   AND (?2 IS NULL OR predicate = ?2)
                 ORDER BY updated_at DESC
                 LIMIT 300",
            )
            .map_err(|e| format!("graph_relations_global prepare: {e}"))?;
        let mut rows = stmt
            .query(params![subject, predicate])
            .map_err(|e| format!("graph_relations_global query: {e}"))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| format!("graph_relations_global row: {e}"))?
        {
            let attrs_raw: String = row.get(3).map_err(|e| e.to_string())?;
            out.push(Self::graph_relation_from_parts(
                None,
                row.get(0).map_err(|e| e.to_string())?,
                row.get(1).map_err(|e| e.to_string())?,
                row.get(2).map_err(|e| e.to_string())?,
                &attrs_raw,
                row.get(4).map_err(|e| e.to_string())?,
            ));
        }
        Ok(out)
    }

    /// Query relations from `graph_namespace` across ALL namespaces, with
    /// optional subject/predicate filters.
    pub(crate) async fn graph_relations_all_namespaces(
        &self,
        subject: Option<&str>,
        predicate: Option<&str>,
    ) -> Result<Vec<GraphRelationRecord>, String> {
        let conn = self.conn.lock();
        let subject = subject.map(Self::normalize_graph_entity);
        let predicate = predicate.map(Self::normalize_graph_predicate);
        let mut stmt = conn
            .prepare(
                "SELECT namespace, subject, predicate, object, attrs_json, updated_at
                 FROM graph_namespace
                 WHERE (?1 IS NULL OR subject = ?1)
                   AND (?2 IS NULL OR predicate = ?2)
                 ORDER BY updated_at DESC
                 LIMIT 300",
            )
            .map_err(|e| format!("graph_relations_all_namespaces prepare: {e}"))?;
        let mut rows = stmt
            .query(params![subject, predicate])
            .map_err(|e| format!("graph_relations_all_namespaces query: {e}"))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| format!("graph_relations_all_namespaces row: {e}"))?
        {
            let namespace: String = row.get(0).map_err(|e| e.to_string())?;
            let attrs_raw: String = row.get(4).map_err(|e| e.to_string())?;
            out.push(Self::graph_relation_from_parts(
                Some(namespace),
                row.get(1).map_err(|e| e.to_string())?,
                row.get(2).map_err(|e| e.to_string())?,
                row.get(3).map_err(|e| e.to_string())?,
                &attrs_raw,
                row.get(5).map_err(|e| e.to_string())?,
            ));
        }
        Ok(out)
    }

    async fn graph_upsert_internal(
        &self,
        namespace: Option<&str>,
        subject: &str,
        predicate: &str,
        object: &str,
        attrs: &serde_json::Value,
    ) -> Result<(), String> {
        let subject = Self::normalize_graph_entity(subject);
        let predicate = Self::normalize_graph_predicate(predicate);
        let object = Self::normalize_graph_entity(object);
        let updated_at = Self::now_ts();
        let conn = self.conn.lock();

        let existing_attrs: Option<String> = match namespace {
            Some(ns) => conn
                .query_row(
                    "SELECT attrs_json
                     FROM graph_namespace
                     WHERE namespace = ?1 AND subject = ?2 AND predicate = ?3 AND object = ?4",
                    params![Self::sanitize_namespace(ns), subject, predicate, object],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| format!("graph_upsert_namespace lookup: {e}"))?,
            None => conn
                .query_row(
                    "SELECT attrs_json
                     FROM graph_global
                     WHERE subject = ?1 AND predicate = ?2 AND object = ?3",
                    params![subject, predicate, object],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| format!("graph_upsert_global lookup: {e}"))?,
        };

        let merged_attrs = Self::merge_graph_attrs(existing_attrs.as_deref(), attrs, updated_at);
        let merged_attrs_json = merged_attrs.to_string();

        match namespace {
            Some(ns) => {
                conn.execute(
                    "INSERT INTO graph_namespace (namespace, subject, predicate, object, attrs_json, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(namespace, subject, predicate, object)
                     DO UPDATE SET attrs_json = excluded.attrs_json, updated_at = excluded.updated_at",
                    params![
                        Self::sanitize_namespace(ns),
                        subject,
                        predicate,
                        object,
                        merged_attrs_json,
                        updated_at
                    ],
                )
                .map_err(|e| format!("graph_upsert_namespace: {e}"))?;
            }
            None => {
                conn.execute(
                    "INSERT INTO graph_global (subject, predicate, object, attrs_json, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(subject, predicate, object)
                     DO UPDATE SET attrs_json = excluded.attrs_json, updated_at = excluded.updated_at",
                    params![subject, predicate, object, merged_attrs_json, updated_at],
                )
                .map_err(|e| format!("graph_upsert_global: {e}"))?;
            }
        }

        Ok(())
    }

    fn merge_graph_attrs(
        existing_attrs_raw: Option<&str>,
        incoming_attrs: &Value,
        updated_at: f64,
    ) -> Value {
        let existing = existing_attrs_raw
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            .unwrap_or_else(|| json!({}));
        let existing_evidence = Self::json_i64(&existing, "evidence_count")
            .unwrap_or(0)
            .max(0) as u64;
        let existing_document_ids =
            Self::json_string_array(&existing, "document_ids", "document_id");
        let existing_chunk_ids = Self::json_string_array(&existing, "chunk_ids", "chunk_id");

        let mut merged = match existing {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        let incoming_map = incoming_attrs.as_object().cloned().unwrap_or_default();
        let existing_order_index = Self::json_i64(&Value::Object(merged.clone()), "order_index");
        let incoming_order_index = Self::json_i64(incoming_attrs, "order_index");
        let merged_order_index = match (existing_order_index, incoming_order_index) {
            (Some(left), Some(right)) => Some(left.min(right)),
            (Some(left), None) => Some(left),
            (None, Some(right)) => Some(right),
            (None, None) => None,
        };

        for (key, value) in incoming_map {
            merged.insert(key, value);
        }

        let incoming_evidence = Self::json_i64(incoming_attrs, "evidence_count")
            .unwrap_or(1)
            .max(0) as u64;
        let evidence_count = existing_evidence.saturating_add(incoming_evidence).max(1);

        merged.insert("evidence_count".to_string(), json!(evidence_count));
        merged.insert("updated_at".to_string(), json!(updated_at));

        let mut document_ids = existing_document_ids;
        document_ids.extend(Self::json_string_array(
            incoming_attrs,
            "document_ids",
            "document_id",
        ));
        document_ids.sort();
        document_ids.dedup();
        if !document_ids.is_empty() {
            merged.insert("document_ids".to_string(), json!(document_ids));
        }

        let mut chunk_ids = existing_chunk_ids;
        chunk_ids.extend(Self::json_string_array(
            incoming_attrs,
            "chunk_ids",
            "chunk_id",
        ));
        chunk_ids.sort();
        chunk_ids.dedup();
        if !chunk_ids.is_empty() {
            merged.insert("chunk_ids".to_string(), json!(chunk_ids));
        }

        if !merged.contains_key("created_at") {
            merged.insert("created_at".to_string(), json!(updated_at));
        }
        if let Some(order_index) = merged_order_index {
            merged.insert("order_index".to_string(), json!(order_index));
        }

        Value::Object(merged)
    }

    fn graph_relation_from_parts(
        namespace: Option<String>,
        subject: String,
        predicate: String,
        object: String,
        attrs_raw: &str,
        updated_at: f64,
    ) -> GraphRelationRecord {
        let attrs = serde_json::from_str::<Value>(attrs_raw).unwrap_or_else(|_| json!({}));
        let evidence_count = Self::json_i64(&attrs, "evidence_count").unwrap_or(1).max(1) as u32;
        let order_index = Self::json_i64(&attrs, "order_index");
        let document_ids = Self::json_string_array(&attrs, "document_ids", "document_id");
        let chunk_ids = Self::json_string_array(&attrs, "chunk_ids", "chunk_id");

        GraphRelationRecord {
            namespace,
            subject,
            predicate,
            object,
            attrs,
            updated_at,
            evidence_count,
            order_index,
            document_ids,
            chunk_ids,
        }
    }

    fn graph_relation_to_json(record: GraphRelationRecord) -> serde_json::Value {
        json!({
            "namespace": record.namespace,
            "subject": record.subject,
            "predicate": record.predicate,
            "object": record.object,
            "attrs": record.attrs,
            "updatedAt": record.updated_at,
            "evidenceCount": record.evidence_count,
            "orderIndex": record.order_index,
            "documentIds": record.document_ids,
            "chunkIds": record.chunk_ids,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::embeddings::NoopEmbedding;
    use std::sync::Arc;
    use tempfile::TempDir;

    #[test]
    fn merge_graph_attrs_accumulates_evidence_and_dedupes_ids() {
        let existing = json!({
            "evidence_count": 2,
            "document_ids": ["doc-1"],
            "chunk_ids": ["doc-1:chunk-1"],
            "order_index": 7,
            "created_at": 1.0
        });
        let incoming = json!({
            "evidence_count": 3,
            "document_ids": ["doc-1", "doc-2"],
            "chunk_ids": ["doc-2:chunk-9"],
            "order_index": 3,
            "attrs_only": true
        });

        let merged = UnifiedMemory::merge_graph_attrs(Some(&existing.to_string()), &incoming, 9.0);
        assert_eq!(merged["evidence_count"], json!(5));
        assert_eq!(merged["document_ids"], json!(["doc-1", "doc-2"]));
        assert_eq!(
            merged["chunk_ids"],
            json!(["doc-1:chunk-1", "doc-2:chunk-9"])
        );
        assert_eq!(merged["order_index"], json!(3));
        assert_eq!(merged["created_at"], json!(1.0));
        assert_eq!(merged["updated_at"], json!(9.0));
        assert_eq!(merged["attrs_only"], json!(true));
    }

    #[test]
    fn graph_relation_from_parts_extracts_counts_and_ids() {
        let record = UnifiedMemory::graph_relation_from_parts(
            Some("global".into()),
            "Alice".into(),
            "OWNS".into(),
            "OpenHuman".into(),
            r#"{"evidence_count":2,"order_index":4,"document_ids":["doc-1"],"chunk_ids":["doc-1:chunk-1"]}"#,
            5.0,
        );
        assert_eq!(record.namespace.as_deref(), Some("global"));
        assert_eq!(record.evidence_count, 2);
        assert_eq!(record.order_index, Some(4));
        assert_eq!(record.document_ids, vec!["doc-1".to_string()]);
        assert_eq!(record.chunk_ids, vec!["doc-1:chunk-1".to_string()]);
    }

    #[test]
    fn merge_graph_attrs_recovers_from_invalid_existing_json_and_negative_evidence() {
        let incoming = json!({
            "evidence_count": -4,
            "document_id": "doc-2",
            "chunk_id": "doc-2:chunk-9",
            "order_index": 8
        });

        let merged = UnifiedMemory::merge_graph_attrs(Some("not-json"), &incoming, 11.0);
        assert_eq!(
            merged["evidence_count"],
            json!(1),
            "negative evidence should clamp to the minimum count"
        );
        assert_eq!(merged["document_ids"], json!(["doc-2"]));
        assert_eq!(merged["chunk_ids"], json!(["doc-2:chunk-9"]));
        assert_eq!(merged["order_index"], json!(8));
        assert_eq!(merged["created_at"], json!(11.0));
        assert_eq!(merged["updated_at"], json!(11.0));
    }

    #[test]
    fn graph_relation_from_parts_defaults_invalid_attrs_payload() {
        let record = UnifiedMemory::graph_relation_from_parts(
            None,
            "Alice".into(),
            "OWNS".into(),
            "Phoenix".into(),
            "not-json",
            7.5,
        );
        assert_eq!(record.evidence_count, 1);
        assert_eq!(record.order_index, None);
        assert!(record.document_ids.is_empty());
        assert!(record.chunk_ids.is_empty());
        assert_eq!(record.attrs, json!({}));
    }

    #[test]
    fn graph_relation_to_json_uses_expected_public_keys() {
        let value = UnifiedMemory::graph_relation_to_json(GraphRelationRecord {
            namespace: None,
            subject: "Alice".into(),
            predicate: "OWNS".into(),
            object: "OpenHuman".into(),
            attrs: json!({"extra": true}),
            updated_at: 1.5,
            evidence_count: 1,
            order_index: Some(2),
            document_ids: vec!["doc-1".into()],
            chunk_ids: vec!["doc-1:chunk-1".into()],
        });
        assert_eq!(value["subject"], "Alice");
        assert_eq!(value["predicate"], "OWNS");
        assert_eq!(value["evidenceCount"], 1);
        assert_eq!(value["orderIndex"], 2);
        assert_eq!(value["documentIds"], json!(["doc-1"]));
        assert_eq!(value["chunkIds"], json!(["doc-1:chunk-1"]));
    }

    fn test_memory() -> (TempDir, UnifiedMemory) {
        let tmp = TempDir::new().unwrap();
        let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();
        (tmp, memory)
    }

    #[tokio::test]
    async fn graph_upsert_namespace_merges_attrs_and_query_returns_json() {
        let (_tmp, memory) = test_memory();
        memory
            .graph_upsert_namespace(
                "team alpha/#1",
                "Alice",
                "OWNS",
                "Phoenix",
                &json!({
                    "document_id": "doc-1",
                    "chunk_id": "doc-1:chunk-1",
                    "evidence_count": 1
                }),
            )
            .await
            .unwrap();
        memory
            .graph_upsert_namespace(
                "team alpha/#1",
                "Alice",
                "OWNS",
                "Phoenix",
                &json!({
                    "document_ids": ["doc-2"],
                    "chunk_ids": ["doc-2:chunk-9"],
                    "order_index": 2
                }),
            )
            .await
            .unwrap();

        let rows = memory
            .graph_query_namespace("team alpha/#1", Some("Alice"), Some("OWNS"))
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["subject"], "ALICE");
        assert_eq!(rows[0]["predicate"], "OWNS");
        assert_eq!(rows[0]["object"], "PHOENIX");
        assert_eq!(rows[0]["evidenceCount"], 2);
        assert_eq!(rows[0]["orderIndex"], 2);
        assert_eq!(rows[0]["documentIds"], json!(["doc-1", "doc-2"]));
        assert_eq!(
            rows[0]["chunkIds"],
            json!(["doc-1:chunk-1", "doc-2:chunk-9"])
        );

        let scoped = memory
            .graph_relations_for_scope("team alpha/#1")
            .await
            .unwrap();
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].namespace.as_deref(), Some("team_alpha/_1"));
    }

    #[tokio::test]
    async fn graph_global_and_all_queries_include_expected_rows() {
        let (_tmp, memory) = test_memory();
        memory
            .graph_upsert_global(
                "Bob",
                "MENTIONED",
                "Launch",
                &json!({"document_id": "doc-global"}),
            )
            .await
            .unwrap();
        memory
            .graph_upsert_namespace(
                "project",
                "Alice",
                "OWNS",
                "Phoenix",
                &json!({"document_id": "doc-local"}),
            )
            .await
            .unwrap();

        let global = memory
            .graph_query_global(Some("Bob"), Some("MENTIONED"))
            .await
            .unwrap();
        assert_eq!(global.len(), 1);
        assert_eq!(global[0]["namespace"], Value::Null);
        assert_eq!(global[0]["subject"], "BOB");

        let all = memory.graph_query_all(None, None).await.unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|row| row["subject"] == "ALICE"));
        assert!(all.iter().any(|row| row["subject"] == "BOB"));
    }

    #[tokio::test]
    async fn graph_relations_for_scope_includes_global_rows_and_sorts_newest_first() {
        let (_tmp, memory) = test_memory();
        memory
            .graph_upsert_namespace(
                "scope-a",
                "Alice",
                "OWNS",
                "Phoenix",
                &json!({"document_id": "doc-local"}),
            )
            .await
            .unwrap();
        memory
            .graph_upsert_global(
                "Bob",
                "MENTIONED",
                "Launch",
                &json!({"document_id": "doc-global"}),
            )
            .await
            .unwrap();

        let scoped = memory.graph_relations_for_scope("scope-a").await.unwrap();
        assert_eq!(scoped.len(), 2);
        assert!(scoped
            .iter()
            .any(|row| row.namespace.as_deref() == Some("scope-a")));
        assert!(scoped.iter().any(|row| row.namespace.is_none()));
        assert!(
            scoped[0].updated_at >= scoped[1].updated_at,
            "scope queries should stay sorted newest-first across namespace+global rows"
        );
    }

    #[tokio::test]
    async fn graph_remove_document_namespace_prunes_or_deletes_relations() {
        let (_tmp, memory) = test_memory();
        memory
            .graph_upsert_namespace(
                "cleanup",
                "Alice",
                "OWNS",
                "Phoenix",
                &json!({
                    "document_ids": ["doc-1", "doc-2"],
                    "chunk_ids": ["doc-1:chunk-1", "doc-2:chunk-2"]
                }),
            )
            .await
            .unwrap();
        memory
            .graph_upsert_namespace(
                "cleanup",
                "Alice",
                "BLOCKED",
                "Atlas",
                &json!({
                    "document_id": "doc-1",
                    "chunk_id": "doc-1:chunk-9"
                }),
            )
            .await
            .unwrap();

        memory
            .graph_remove_document_namespace("cleanup", "doc-1")
            .await
            .unwrap();

        let rows = memory
            .graph_query_namespace("cleanup", None, None)
            .await
            .unwrap();
        assert_eq!(
            rows.len(),
            1,
            "single-doc relation should be deleted entirely"
        );
        assert_eq!(rows[0]["predicate"], "OWNS");
        assert_eq!(rows[0]["documentIds"], json!(["doc-2"]));
        assert_eq!(rows[0]["chunkIds"], json!(["doc-2:chunk-2"]));
    }

    #[tokio::test]
    async fn graph_remove_document_namespace_is_noop_for_unrelated_document() {
        let (_tmp, memory) = test_memory();
        memory
            .graph_upsert_namespace(
                "cleanup",
                "Alice",
                "OWNS",
                "Phoenix",
                &json!({
                    "document_ids": ["doc-2"],
                    "chunk_ids": ["doc-2:chunk-2"]
                }),
            )
            .await
            .unwrap();

        memory
            .graph_remove_document_namespace("cleanup", "doc-missing")
            .await
            .unwrap();

        let rows = memory
            .graph_query_namespace("cleanup", None, None)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["documentIds"], json!(["doc-2"]));
        assert_eq!(rows[0]["chunkIds"], json!(["doc-2:chunk-2"]));
    }
}
