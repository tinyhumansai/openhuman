use rusqlite::{params, OptionalExtension};
use serde_json::{json, Map, Value};

use crate::openhuman::memory::store::types::GraphRelationRecord;

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
