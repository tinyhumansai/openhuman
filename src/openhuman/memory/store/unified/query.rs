use rusqlite::params;
use std::collections::HashMap;

use crate::openhuman::memory::store::types::NamespaceQueryResult;

use super::UnifiedMemory;

impl UnifiedMemory {
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
}
