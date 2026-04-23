use rusqlite::params;
use std::collections::{HashMap, HashSet};

use crate::openhuman::memory::store::types::{
    GraphRelationRecord, MemoryItemKind, NamespaceMemoryHit, NamespaceQueryResult,
    NamespaceRetrievalContext, RetrievalScoreBreakdown,
};

use super::events;
use super::fts5;
use super::UnifiedMemory;

const GRAPH_WEIGHT: f64 = 0.55;
const VECTOR_WEIGHT: f64 = 0.30;
const KEYWORD_WEIGHT: f64 = 0.15;
const EPISODIC_WEIGHT: f64 = 0.20;

// Adjusted weights when episodic signal is present
const GRAPH_WEIGHT_WITH_EPISODIC: f64 = 0.45;
const VECTOR_WEIGHT_WITH_EPISODIC: f64 = 0.25;
const KEYWORD_WEIGHT_WITH_EPISODIC: f64 = 0.10;

const RECALL_PRIORITY_WEIGHT: f64 = 0.45;
const RECALL_GRAPH_WEIGHT: f64 = 0.30;
const RECALL_FRESHNESS_WEIGHT: f64 = 0.25;

#[derive(Debug, Clone)]
struct StoredChunk {
    document_id: String,
    chunk_id: String,
    text: String,
    embedding: Option<Vec<f32>>,
    updated_at: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TemporalOperator {
    Latest,
    Earliest,
    Before,
    After,
    All,
}

#[derive(Debug, Clone)]
struct RetrievalPlan {
    query_terms: Vec<String>,
    seed_entities: Vec<String>,
    relation_types: Vec<String>,
    chains: Vec<Vec<String>>,
    temporal: TemporalOperator,
    anchor_entity: Option<String>,
}

#[derive(Debug, Clone)]
struct RelationMatch {
    relation: GraphRelationRecord,
    hop: usize,
}

impl UnifiedMemory {
    /// Relation-first retrieval:
    /// - graph relevance is the primary signal
    /// - vector similarity is the secondary verification signal
    /// - keyword overlap remains as a lexical backstop
    pub async fn query_namespace_ranked(
        &self,
        namespace: &str,
        query: &str,
        limit: u32,
    ) -> Result<Vec<NamespaceQueryResult>, String> {
        let hits = self.query_namespace_hits(namespace, query, limit).await?;
        let mut out = Vec::new();
        for hit in hits {
            if hit.kind != MemoryItemKind::Document {
                continue;
            }
            out.push(NamespaceQueryResult {
                key: hit.key,
                content: hit.content,
                score: hit.score,
                category: hit.category,
            });
        }
        Ok(out)
    }

    pub async fn query_namespace_hits(
        &self,
        namespace: &str,
        query: &str,
        limit: u32,
    ) -> Result<Vec<NamespaceMemoryHit>, String> {
        let ns = Self::sanitize_namespace(namespace);
        let docs = self.load_documents_for_scope(&ns).await?;
        let kvs = self.kv_records_for_scope(&ns).await?;

        let graph_relations = self
            .graph_relations_for_scope(&ns)
            .await
            .unwrap_or_default();
        let chunks = self.load_chunks_for_scope(&ns).await?;
        let plan = self.build_retrieval_plan(query, &docs, &graph_relations);
        let matched_relations = self.collect_relation_matches(&plan, &graph_relations);
        let graph_scores = self.compute_graph_document_scores(&docs, &chunks, &matched_relations);
        let vector_scores = self
            .query_vector_scores_from_chunks(&chunks, query)
            .await
            .unwrap_or_default();
        let query_terms = plan.query_terms.clone();
        let now = Self::now_ts();

        let has_graph_signal = graph_scores.values().any(|score| *score > 0.0);
        let mut hits = Vec::new();

        for doc in docs {
            let keyword = self.keyword_score_for_text(
                &query_terms,
                &[doc.key.as_str(), doc.title.as_str(), doc.content.as_str()],
            );
            let vector = vector_scores
                .get(&doc.document_id)
                .map(|(score, _)| *score)
                .unwrap_or(0.0);
            let graph = graph_scores.get(&doc.document_id).copied().unwrap_or(0.0);
            let breakdown = if has_graph_signal {
                Self::compose_query_score(keyword, vector, graph)
            } else {
                Self::compose_fallback_query_score(keyword, vector)
            };
            if breakdown.final_score <= 0.0 {
                continue;
            }

            let best_chunk_id = vector_scores
                .get(&doc.document_id)
                .and_then(|(_, chunk_id)| chunk_id.clone());
            let supporting_relations = self.supporting_relations_for_document(
                &doc.document_id,
                &doc.content,
                &matched_relations,
            );

            hits.push(NamespaceMemoryHit {
                id: doc.document_id.clone(),
                kind: MemoryItemKind::Document,
                namespace: doc.namespace.clone(),
                key: doc.key.clone(),
                title: Some(doc.title.clone()),
                content: doc.content.clone(),
                category: doc.category.clone(),
                source_type: Some(doc.source_type.clone()),
                updated_at: doc.updated_at,
                score: breakdown.final_score,
                score_breakdown: breakdown,
                document_id: Some(doc.document_id.clone()),
                chunk_id: best_chunk_id,
                supporting_relations,
            });
        }

        for kv in kvs {
            let rendered = Self::render_kv_value(&kv.value);
            let keyword =
                self.keyword_score_for_text(&query_terms, &[kv.key.as_str(), rendered.as_str()]);
            if keyword <= 0.0 {
                continue;
            }
            let freshness = Self::recency_score(kv.updated_at, now);
            let final_score = (keyword * 0.8) + (freshness * 0.2);
            hits.push(NamespaceMemoryHit {
                id: format!(
                    "kv:{}:{}",
                    kv.namespace.as_deref().unwrap_or("global"),
                    kv.key
                ),
                kind: MemoryItemKind::Kv,
                namespace: kv.namespace.unwrap_or_else(|| "global".to_string()),
                key: kv.key,
                title: None,
                content: rendered,
                category: "kv".to_string(),
                source_type: None,
                updated_at: kv.updated_at,
                score: final_score,
                score_breakdown: RetrievalScoreBreakdown {
                    keyword_relevance: keyword,
                    vector_similarity: 0.0,
                    graph_relevance: 0.0,
                    episodic_relevance: 0.0,
                    freshness,
                    final_score,
                },
                document_id: None,
                chunk_id: None,
                supporting_relations: Vec::new(),
            });
        }

        // Episodic FTS5 search — search past conversation turns.
        // Only merge episodic results when querying the global namespace,
        // since episodic entries are session-scoped, not namespace-scoped.
        let episodic_hits = if ns == "global" {
            fts5::episodic_search(&self.conn, query, limit as usize).unwrap_or_else(|e| {
                tracing::warn!("[query] episodic search failed: {e}");
                Vec::new()
            })
        } else {
            Vec::new()
        };

        if !episodic_hits.is_empty() {
            tracing::debug!(
                "[query] merging {} episodic hits for '{}'",
                episodic_hits.len(),
                query
            );

            // Reweight existing document/KV hits when episodic signal is present.
            let has_episodic = true;
            if has_episodic {
                for hit in &mut hits {
                    if hit.kind == MemoryItemKind::Document {
                        let bd = &hit.score_breakdown;
                        let new_score = (bd.graph_relevance * GRAPH_WEIGHT_WITH_EPISODIC)
                            + (bd.vector_similarity * VECTOR_WEIGHT_WITH_EPISODIC)
                            + (bd.keyword_relevance * KEYWORD_WEIGHT_WITH_EPISODIC);
                        hit.score = new_score;
                        hit.score_breakdown.final_score = new_score;
                    }
                }
            }

            for entry in &episodic_hits {
                let freshness = Self::recency_score(entry.timestamp, now);
                // Episodic FTS5 returns results ordered by rank (best first).
                // Normalize position to a 0-1 relevance score.
                let position_idx = episodic_hits
                    .iter()
                    .position(|e| e.id == entry.id)
                    .unwrap_or(0);
                let fts_relevance = 1.0 - (position_idx as f64 / episodic_hits.len().max(1) as f64);

                let episodic_score = (fts_relevance * 0.7) + (freshness * 0.3);
                let final_score = episodic_score * EPISODIC_WEIGHT;

                // Truncate long episodic content for context display (UTF-8 safe).
                let content = match entry.content.char_indices().nth(500) {
                    Some((byte_idx, _)) => format!("{}...", &entry.content[..byte_idx]),
                    None => entry.content.clone(),
                };

                hits.push(NamespaceMemoryHit {
                    id: format!("episodic:{}", entry.id.unwrap_or(0)),
                    kind: MemoryItemKind::Episodic,
                    namespace: ns.clone(),
                    key: format!("{}:{}", entry.session_id, entry.role),
                    title: entry.lesson.clone(),
                    content,
                    category: "episodic".to_string(),
                    source_type: Some(entry.role.clone()),
                    updated_at: entry.timestamp,
                    score: final_score,
                    score_breakdown: RetrievalScoreBreakdown {
                        keyword_relevance: 0.0,
                        vector_similarity: 0.0,
                        graph_relevance: 0.0,
                        episodic_relevance: fts_relevance,
                        freshness,
                        final_score,
                    },
                    document_id: None,
                    chunk_id: None,
                    supporting_relations: Vec::new(),
                });
            }
        }

        // Event FTS5 search — search extracted facts, decisions, preferences.
        let event_hits = events::event_search_fts(&self.conn, &ns, query, limit as usize)
            .unwrap_or_else(|e| {
                tracing::warn!("[query] event search failed: {e}");
                Vec::new()
            });

        for (idx, event) in event_hits.iter().enumerate() {
            let freshness = Self::recency_score(event.created_at, now);
            let fts_relevance = 1.0 - (idx as f64 / event_hits.len().max(1) as f64);
            let final_score = (fts_relevance * 0.6) + (freshness * 0.4);

            hits.push(NamespaceMemoryHit {
                id: format!("event:{}", event.event_id),
                kind: MemoryItemKind::Event,
                namespace: event.namespace.clone(),
                key: format!("{}:{}", event.event_type.as_str(), event.segment_id),
                title: event.subject.clone(),
                content: event.content.clone(),
                category: event.event_type.as_str().to_string(),
                source_type: Some("event".to_string()),
                updated_at: event.created_at,
                score: final_score,
                score_breakdown: RetrievalScoreBreakdown {
                    keyword_relevance: fts_relevance,
                    vector_similarity: 0.0,
                    graph_relevance: 0.0,
                    episodic_relevance: 0.0,
                    freshness,
                    final_score,
                },
                document_id: None,
                chunk_id: None,
                supporting_relations: Vec::new(),
            });
        }

        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(limit as usize);
        Ok(hits)
    }

    pub async fn query_namespace_context(
        &self,
        namespace: &str,
        query: &str,
        limit: u32,
    ) -> Result<String, String> {
        let context = self
            .query_namespace_context_data(namespace, query, limit)
            .await?;
        Ok(context.context_text)
    }

    pub async fn query_namespace_context_data(
        &self,
        namespace: &str,
        query: &str,
        limit: u32,
    ) -> Result<NamespaceRetrievalContext, String> {
        let ns = Self::sanitize_namespace(namespace);
        let hits = self.query_namespace_hits(&ns, query, limit).await?;
        Ok(NamespaceRetrievalContext {
            namespace: ns,
            query: Some(query.to_string()),
            context_text: Self::format_context_text(&hits, Some(query)),
            hits,
        })
    }

    pub async fn recall_namespace_memories(
        &self,
        namespace: &str,
        limit: u32,
    ) -> Result<Vec<NamespaceMemoryHit>, String> {
        let ns = Self::sanitize_namespace(namespace);
        let docs = self.load_documents_for_scope(&ns).await?;
        let kvs = self.kv_records_for_scope(&ns).await?;
        let graph_relations = self
            .graph_relations_for_scope(&ns)
            .await
            .unwrap_or_default();
        let now = Self::now_ts();
        let mut hits = Vec::new();

        for doc in docs {
            let freshness = Self::recency_score(doc.updated_at, now);
            let priority = Self::document_priority_signal(
                &doc.category,
                &doc.priority,
                &doc.tags,
                &doc.metadata,
            );
            let graph =
                self.document_recall_graph_signal(&doc.document_id, &doc.content, &graph_relations);
            let final_score = (priority * RECALL_PRIORITY_WEIGHT)
                + (graph * RECALL_GRAPH_WEIGHT)
                + (freshness * RECALL_FRESHNESS_WEIGHT);
            hits.push(NamespaceMemoryHit {
                id: doc.document_id.clone(),
                kind: MemoryItemKind::Document,
                namespace: doc.namespace.clone(),
                key: doc.key.clone(),
                title: Some(doc.title.clone()),
                content: doc.content.clone(),
                category: doc.category.clone(),
                source_type: Some(doc.source_type.clone()),
                updated_at: doc.updated_at,
                score: final_score,
                score_breakdown: RetrievalScoreBreakdown {
                    keyword_relevance: priority,
                    vector_similarity: 0.0,
                    graph_relevance: graph,
                    episodic_relevance: 0.0,
                    freshness,
                    final_score,
                },
                document_id: Some(doc.document_id.clone()),
                chunk_id: None,
                supporting_relations: self.supporting_relations_for_document(
                    &doc.document_id,
                    &doc.content,
                    &graph_relations
                        .iter()
                        .cloned()
                        .map(|relation| RelationMatch { relation, hop: 1 })
                        .collect::<Vec<_>>(),
                ),
            });
        }

        for kv in kvs {
            let freshness = Self::recency_score(kv.updated_at, now);
            let priority = Self::kv_priority_signal(&kv.key, &kv.value);
            let final_score =
                (priority * RECALL_PRIORITY_WEIGHT) + (freshness * (1.0 - RECALL_PRIORITY_WEIGHT));
            hits.push(NamespaceMemoryHit {
                id: format!(
                    "kv:{}:{}",
                    kv.namespace.as_deref().unwrap_or("global"),
                    kv.key
                ),
                kind: MemoryItemKind::Kv,
                namespace: kv.namespace.unwrap_or_else(|| "global".to_string()),
                key: kv.key,
                title: None,
                content: Self::render_kv_value(&kv.value),
                category: "kv".to_string(),
                source_type: None,
                updated_at: kv.updated_at,
                score: final_score,
                score_breakdown: RetrievalScoreBreakdown {
                    keyword_relevance: priority,
                    vector_similarity: 0.0,
                    graph_relevance: 0.0,
                    episodic_relevance: 0.0,
                    freshness,
                    final_score,
                },
                document_id: None,
                chunk_id: None,
                supporting_relations: Vec::new(),
            });
        }

        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(limit as usize);
        Ok(hits)
    }

    pub async fn recall_namespace_context(
        &self,
        namespace: &str,
        max_chunks: u32,
    ) -> Result<Option<String>, String> {
        let hits = self
            .recall_namespace_memories(namespace, max_chunks)
            .await?;
        if hits.is_empty() {
            return Ok(None);
        }
        Ok(Some(Self::format_context_text(&hits, None)))
    }

    pub async fn recall_namespace_context_data(
        &self,
        namespace: &str,
        limit: u32,
    ) -> Result<NamespaceRetrievalContext, String> {
        let ns = Self::sanitize_namespace(namespace);
        let hits = self.recall_namespace_memories(&ns, limit).await?;
        Ok(NamespaceRetrievalContext {
            namespace: ns,
            query: None,
            context_text: Self::format_context_text(&hits, None),
            hits,
        })
    }

    async fn load_chunks_for_scope(&self, namespace: &str) -> Result<Vec<StoredChunk>, String> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT document_id, chunk_id, text, embedding, updated_at
                 FROM vector_chunks
                 WHERE namespace = ?1",
            )
            .map_err(|e| format!("prepare load_chunks_for_scope: {e}"))?;
        let mut rows = stmt
            .query(params![Self::sanitize_namespace(namespace)])
            .map_err(|e| format!("query load_chunks_for_scope: {e}"))?;
        let mut chunks = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| format!("row load_chunks_for_scope: {e}"))?
        {
            let embedding_blob: Option<Vec<u8>> = row.get(3).map_err(|e| e.to_string())?;
            chunks.push(StoredChunk {
                document_id: row.get(0).map_err(|e| e.to_string())?,
                chunk_id: row.get(1).map_err(|e| e.to_string())?,
                text: row.get(2).map_err(|e| e.to_string())?,
                embedding: embedding_blob.as_deref().map(Self::bytes_to_vec),
                updated_at: row.get(4).map_err(|e| e.to_string())?,
            });
        }
        Ok(chunks)
    }

    async fn query_vector_scores_from_chunks(
        &self,
        chunks: &[StoredChunk],
        query: &str,
    ) -> Result<HashMap<String, (f64, Option<String>)>, String> {
        if chunks.is_empty() {
            return Ok(HashMap::new());
        }
        let query_embedding = self
            .embedder
            .embed_one(query)
            .await
            .map_err(|e| format!("embedding query: {e}"))?;
        let mut scores = HashMap::new();
        for chunk in chunks {
            let Some(embedding) = chunk.embedding.as_ref() else {
                continue;
            };
            let similarity = Self::cosine_similarity(&query_embedding, embedding);
            let entry = scores
                .entry(chunk.document_id.clone())
                .or_insert((0.0, None::<String>));
            if similarity > entry.0 {
                *entry = (similarity, Some(chunk.chunk_id.clone()));
            }
        }
        Ok(scores)
    }

    fn build_retrieval_plan(
        &self,
        query: &str,
        docs: &[crate::openhuman::memory::store::types::StoredMemoryDocument],
        graph_relations: &[GraphRelationRecord],
    ) -> RetrievalPlan {
        let query_terms = Self::tokenize_search_terms(query);
        let temporal = Self::infer_temporal_operator(&query_terms);
        let relation_types = Self::infer_relation_types(&query_terms);
        let entity_candidates = self.match_query_entities(query, docs, graph_relations);
        let anchor_entity = match temporal {
            TemporalOperator::Before | TemporalOperator::After => {
                self.resolve_anchor_entity(query, &entity_candidates)
            }
            _ => None,
        };
        let seed_entities = entity_candidates
            .into_iter()
            .filter(|entity| anchor_entity.as_ref() != Some(entity))
            .collect::<Vec<_>>();
        let chains = Self::infer_relation_chains(&query_terms, &relation_types);

        RetrievalPlan {
            query_terms,
            seed_entities,
            relation_types,
            chains,
            temporal,
            anchor_entity,
        }
    }

    fn match_query_entities(
        &self,
        query: &str,
        docs: &[crate::openhuman::memory::store::types::StoredMemoryDocument],
        graph_relations: &[GraphRelationRecord],
    ) -> Vec<String> {
        let normalized_query = Self::normalize_search_text(query);
        let mut entities = HashSet::new();

        for relation in graph_relations {
            for candidate in [&relation.subject, &relation.object] {
                let normalized = Self::normalize_search_text(candidate);
                if !normalized.is_empty() && normalized_query.contains(&normalized) {
                    entities.insert(candidate.clone());
                }
            }
        }

        for doc in docs {
            for candidate in [&doc.key, &doc.title] {
                let normalized = Self::normalize_search_text(candidate);
                if !normalized.is_empty() && normalized_query.contains(&normalized) {
                    entities.insert(Self::normalize_graph_entity(candidate));
                }
            }
        }

        let mut out = entities.into_iter().collect::<Vec<_>>();
        out.sort();
        out
    }

    fn resolve_anchor_entity(&self, query: &str, entities: &[String]) -> Option<String> {
        let normalized_query = Self::normalize_search_text(query);
        let mut best: Option<(usize, String)> = None;
        for entity in entities {
            let normalized_entity = Self::normalize_search_text(entity);
            if normalized_entity.is_empty() {
                continue;
            }
            if let Some(pos) = normalized_query.rfind(&normalized_entity) {
                if best
                    .as_ref()
                    .map(|(best_pos, _)| pos > *best_pos)
                    .unwrap_or(true)
                {
                    best = Some((pos, entity.clone()));
                }
            }
        }
        best.map(|(_, entity)| entity)
    }

    fn collect_relation_matches(
        &self,
        plan: &RetrievalPlan,
        graph_relations: &[GraphRelationRecord],
    ) -> Vec<RelationMatch> {
        let matches = self.direct_relation_matches(plan, graph_relations);
        let chain_matches = self.multi_hop_relation_matches(plan, graph_relations);
        let mut merged = matches;
        for item in chain_matches {
            let identity = Self::relation_identity(&item.relation);
            if merged
                .iter()
                .any(|existing| Self::relation_identity(&existing.relation) == identity)
            {
                continue;
            }
            merged.push(item);
        }

        let anchor_order = self.resolve_anchor_order(plan, graph_relations);
        Self::apply_temporal_filter(plan, anchor_order, merged)
    }

    fn direct_relation_matches(
        &self,
        plan: &RetrievalPlan,
        graph_relations: &[GraphRelationRecord],
    ) -> Vec<RelationMatch> {
        let seed_entities = plan.seed_entities.iter().collect::<HashSet<_>>();
        graph_relations
            .iter()
            .filter(|relation| {
                let touches_seed = seed_entities.is_empty()
                    || seed_entities.contains(&relation.subject)
                    || seed_entities.contains(&relation.object);
                let predicate_match = plan.relation_types.is_empty()
                    || plan.relation_types.contains(&relation.predicate)
                    || Self::predicate_matches_query(&relation.predicate, &plan.query_terms);
                let entity_overlap = seed_entities.is_empty()
                    || Self::relation_matches_terms(relation, &plan.query_terms);
                touches_seed && predicate_match && entity_overlap
            })
            .cloned()
            .map(|relation| RelationMatch { relation, hop: 1 })
            .collect()
    }

    fn multi_hop_relation_matches(
        &self,
        plan: &RetrievalPlan,
        graph_relations: &[GraphRelationRecord],
    ) -> Vec<RelationMatch> {
        if plan.chains.is_empty() || plan.seed_entities.is_empty() {
            return Vec::new();
        }

        let mut chain_results: Vec<Vec<RelationMatch>> = Vec::new();
        for chain in &plan.chains {
            let mut frontier = plan.seed_entities.clone();
            let mut path = Vec::new();
            let mut used = HashSet::new();

            for (hop_idx, step) in chain.iter().enumerate() {
                let mut candidates = graph_relations
                    .iter()
                    .filter(|relation| {
                        relation.predicate == *step
                            && (frontier.contains(&relation.subject)
                                || frontier.contains(&relation.object))
                    })
                    .cloned()
                    .collect::<Vec<_>>();

                if candidates.is_empty() {
                    path.clear();
                    break;
                }

                candidates.sort_by(|a, b| {
                    Self::relation_order_value(b)
                        .cmp(&Self::relation_order_value(a))
                        .then_with(|| {
                            b.updated_at
                                .partial_cmp(&a.updated_at)
                                .unwrap_or(std::cmp::Ordering::Equal)
                        })
                });

                let mut next_frontier = Vec::new();
                for relation in candidates {
                    let identity = Self::relation_identity(&relation);
                    if !used.insert(identity) {
                        continue;
                    }
                    if frontier.contains(&relation.subject) {
                        next_frontier.push(relation.object.clone());
                    }
                    if frontier.contains(&relation.object) {
                        next_frontier.push(relation.subject.clone());
                    }
                    path.push(RelationMatch {
                        relation,
                        hop: hop_idx + 2,
                    });
                }

                next_frontier.sort();
                next_frontier.dedup();
                frontier = next_frontier;
            }

            if !path.is_empty() {
                chain_results.push(path);
            }
        }

        if chain_results.is_empty() {
            return Vec::new();
        }

        if plan.temporal == TemporalOperator::All {
            return chain_results.into_iter().flatten().collect();
        }

        let choose_max = matches!(
            plan.temporal,
            TemporalOperator::Latest | TemporalOperator::Before
        );
        chain_results
            .into_iter()
            .max_by(|a, b| {
                let a_order = a
                    .iter()
                    .map(|item| Self::relation_order_value(&item.relation))
                    .max()
                    .unwrap_or_default();
                let b_order = b
                    .iter()
                    .map(|item| Self::relation_order_value(&item.relation))
                    .max()
                    .unwrap_or_default();
                if choose_max {
                    a_order.cmp(&b_order)
                } else {
                    b_order.cmp(&a_order)
                }
            })
            .unwrap_or_default()
    }

    fn apply_temporal_filter(
        plan: &RetrievalPlan,
        anchor_order: Option<i64>,
        relations: Vec<RelationMatch>,
    ) -> Vec<RelationMatch> {
        if plan.temporal == TemporalOperator::All {
            return relations;
        }

        let filtered = match plan.temporal {
            TemporalOperator::Before => relations
                .into_iter()
                .filter(|item| {
                    anchor_order
                        .map(|anchor| Self::relation_order_value(&item.relation) < anchor)
                        .unwrap_or(true)
                })
                .collect::<Vec<_>>(),
            TemporalOperator::After => relations
                .into_iter()
                .filter(|item| {
                    anchor_order
                        .map(|anchor| Self::relation_order_value(&item.relation) > anchor)
                        .unwrap_or(true)
                })
                .collect::<Vec<_>>(),
            _ => relations,
        };

        let mut groups: HashMap<(String, String), Vec<RelationMatch>> = HashMap::new();
        for item in filtered {
            let pivot = if plan.seed_entities.contains(&item.relation.subject) {
                item.relation.subject.clone()
            } else if plan.seed_entities.contains(&item.relation.object) {
                item.relation.object.clone()
            } else {
                item.relation.subject.clone()
            };
            groups
                .entry((pivot, item.relation.predicate.clone()))
                .or_default()
                .push(item);
        }

        let mut out = Vec::new();
        for mut items in groups.into_values() {
            items.sort_by(|a, b| {
                Self::relation_order_value(&a.relation)
                    .cmp(&Self::relation_order_value(&b.relation))
            });
            match plan.temporal {
                TemporalOperator::Earliest | TemporalOperator::After => {
                    if let Some(item) = items.into_iter().next() {
                        out.push(item);
                    }
                }
                TemporalOperator::Latest | TemporalOperator::Before => {
                    if let Some(item) = items.into_iter().last() {
                        out.push(item);
                    }
                }
                TemporalOperator::All => out.extend(items),
            }
        }

        out
    }

    fn resolve_anchor_order(
        &self,
        plan: &RetrievalPlan,
        graph_relations: &[GraphRelationRecord],
    ) -> Option<i64> {
        let anchor = plan.anchor_entity.as_ref()?;
        let mut orders = graph_relations
            .iter()
            .filter(|relation| relation.subject == *anchor || relation.object == *anchor)
            .map(Self::relation_order_value)
            .collect::<Vec<_>>();
        if orders.is_empty() {
            return None;
        }
        orders.sort();
        match plan.temporal {
            TemporalOperator::Before => orders.into_iter().max(),
            TemporalOperator::After => orders.into_iter().min(),
            _ => orders.into_iter().max(),
        }
    }

    fn compute_graph_document_scores(
        &self,
        docs: &[crate::openhuman::memory::store::types::StoredMemoryDocument],
        chunks: &[StoredChunk],
        relations: &[RelationMatch],
    ) -> HashMap<String, f64> {
        let mut doc_scores: HashMap<String, f64> = HashMap::new();
        let chunk_to_doc = chunks
            .iter()
            .map(|chunk| (chunk.chunk_id.clone(), chunk.document_id.clone()))
            .collect::<HashMap<_, _>>();

        for relation in relations {
            let base = f64::from(relation.relation.evidence_count) / relation.hop.max(1) as f64;
            for document_id in &relation.relation.document_ids {
                *doc_scores.entry(document_id.clone()).or_insert(0.0) += base;
            }
            for chunk_id in &relation.relation.chunk_ids {
                if let Some(document_id) = chunk_to_doc.get(chunk_id) {
                    *doc_scores.entry(document_id.clone()).or_insert(0.0) += base * 0.9;
                }
            }

            for doc in docs {
                let normalized = Self::normalize_search_text(&doc.content);
                let subject = Self::normalize_search_text(&relation.relation.subject);
                let object = Self::normalize_search_text(&relation.relation.object);
                if (!subject.is_empty() && normalized.contains(&subject))
                    || (!object.is_empty() && normalized.contains(&object))
                {
                    *doc_scores.entry(doc.document_id.clone()).or_insert(0.0) += base * 0.35;
                }
            }
        }

        Self::normalize_scores(doc_scores)
    }

    fn supporting_relations_for_document(
        &self,
        document_id: &str,
        content: &str,
        relations: &[RelationMatch],
    ) -> Vec<GraphRelationRecord> {
        let normalized_content = Self::normalize_search_text(content);
        let mut out = relations
            .iter()
            .filter(|relation| {
                relation
                    .relation
                    .document_ids
                    .iter()
                    .any(|id| id == document_id)
                    || relation
                        .relation
                        .chunk_ids
                        .iter()
                        .any(|chunk_id| chunk_id.starts_with(document_id))
                    || normalized_content
                        .contains(&Self::normalize_search_text(&relation.relation.subject))
                    || normalized_content
                        .contains(&Self::normalize_search_text(&relation.relation.object))
            })
            .map(|relation| relation.relation.clone())
            .collect::<Vec<_>>();
        out.sort_by(|a, b| {
            b.evidence_count.cmp(&a.evidence_count).then_with(|| {
                b.updated_at
                    .partial_cmp(&a.updated_at)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });
        out.truncate(3);
        out
    }

    fn document_recall_graph_signal(
        &self,
        document_id: &str,
        content: &str,
        relations: &[GraphRelationRecord],
    ) -> f64 {
        let normalized_content = Self::normalize_search_text(content);
        let mut score = 0.0;
        for relation in relations {
            if relation.document_ids.iter().any(|id| id == document_id) {
                score += f64::from(relation.evidence_count);
                continue;
            }
            let subject = Self::normalize_search_text(&relation.subject);
            let object = Self::normalize_search_text(&relation.object);
            if (!subject.is_empty() && normalized_content.contains(&subject))
                || (!object.is_empty() && normalized_content.contains(&object))
            {
                score += f64::from(relation.evidence_count) * 0.35;
            }
        }
        score.clamp(0.0, 10.0) / 10.0
    }

    fn keyword_score_for_text(&self, query_terms: &[String], text_parts: &[&str]) -> f64 {
        if query_terms.is_empty() {
            return 0.0;
        }
        let haystack = text_parts
            .iter()
            .map(|part| Self::normalize_search_text(part))
            .collect::<Vec<_>>()
            .join(" ");
        if haystack.is_empty() {
            return 0.0;
        }
        let matched = query_terms
            .iter()
            .filter(|term| haystack.contains(term.as_str()))
            .count();
        matched as f64 / query_terms.len().max(1) as f64
    }

    fn compose_query_score(
        keyword_relevance: f64,
        vector_similarity: f64,
        graph_relevance: f64,
    ) -> RetrievalScoreBreakdown {
        let final_score = (graph_relevance * GRAPH_WEIGHT)
            + (vector_similarity * VECTOR_WEIGHT)
            + (keyword_relevance * KEYWORD_WEIGHT);
        RetrievalScoreBreakdown {
            keyword_relevance,
            vector_similarity,
            graph_relevance,
            episodic_relevance: 0.0,
            freshness: 0.0,
            final_score,
        }
    }

    fn compose_fallback_query_score(
        keyword_relevance: f64,
        vector_similarity: f64,
    ) -> RetrievalScoreBreakdown {
        let final_score = (vector_similarity * 0.65) + (keyword_relevance * 0.35);
        RetrievalScoreBreakdown {
            keyword_relevance,
            vector_similarity,
            graph_relevance: 0.0,
            episodic_relevance: 0.0,
            freshness: 0.0,
            final_score,
        }
    }

    fn normalize_scores(scores: HashMap<String, f64>) -> HashMap<String, f64> {
        let max_score = scores.values().copied().fold(0.0_f64, f64::max);
        if max_score <= f64::EPSILON {
            return HashMap::new();
        }
        scores
            .into_iter()
            .map(|(key, score)| (key, (score / max_score).clamp(0.0, 1.0)))
            .collect()
    }

    fn infer_temporal_operator(query_terms: &[String]) -> TemporalOperator {
        if query_terms.iter().any(|term| term == "before") {
            TemporalOperator::Before
        } else if query_terms.iter().any(|term| term == "after") {
            TemporalOperator::After
        } else if query_terms
            .iter()
            .any(|term| matches!(term.as_str(), "history" | "timeline" | "all"))
        {
            TemporalOperator::All
        } else if query_terms
            .iter()
            .any(|term| matches!(term.as_str(), "first" | "earliest" | "initial"))
        {
            TemporalOperator::Earliest
        } else {
            TemporalOperator::Latest
        }
    }

    fn infer_relation_types(query_terms: &[String]) -> Vec<String> {
        let mut relation_types = HashSet::new();
        for term in query_terms {
            match term.as_str() {
                "where" | "location" | "located" | "place" => {
                    relation_types.insert("LOCATED_IN".to_string());
                    relation_types.insert("RESIDES_AT".to_string());
                    relation_types.insert("TRAVELS_TO".to_string());
                }
                "owner" | "owns" | "owned" | "has" | "holding" => {
                    relation_types.insert("OWNS".to_string());
                    relation_types.insert("USES".to_string());
                }
                "works" | "employer" | "company" | "organization" => {
                    relation_types.insert("WORKS_FOR".to_string());
                }
                "north" => {
                    relation_types.insert("NORTH_OF".to_string());
                }
                "south" => {
                    relation_types.insert("SOUTH_OF".to_string());
                }
                "east" => {
                    relation_types.insert("EAST_OF".to_string());
                }
                "west" => {
                    relation_types.insert("WEST_OF".to_string());
                }
                "give" | "gave" | "sent" | "handed" | "passed" | "received" | "receive" => {
                    relation_types.insert("USES".to_string());
                }
                _ => {}
            }
        }
        let mut out = relation_types.into_iter().collect::<Vec<_>>();
        out.sort();
        out
    }

    fn infer_relation_chains(
        query_terms: &[String],
        relation_types: &[String],
    ) -> Vec<Vec<String>> {
        let mut chains = Vec::new();
        let asks_where = query_terms.iter().any(|term| term == "where");
        let transfer_like = query_terms.iter().any(|term| {
            matches!(
                term.as_str(),
                "give" | "gave" | "sent" | "handed" | "passed"
            )
        });

        if asks_where {
            chains.push(vec!["OWNS".to_string(), "TRAVELS_TO".to_string()]);
            chains.push(vec!["USES".to_string(), "TRAVELS_TO".to_string()]);
            chains.push(vec!["OWNS".to_string(), "LOCATED_IN".to_string()]);
            chains.push(vec!["USES".to_string(), "LOCATED_IN".to_string()]);
        } else if transfer_like {
            chains.push(vec!["USES".to_string()]);
        } else if !relation_types.is_empty() {
            chains.push(relation_types.to_vec());
        }

        chains.truncate(4);
        chains
    }

    fn predicate_matches_query(predicate: &str, query_terms: &[String]) -> bool {
        let normalized = Self::normalize_search_text(predicate);
        query_terms.iter().any(|term| normalized.contains(term))
    }

    fn relation_matches_terms(relation: &GraphRelationRecord, query_terms: &[String]) -> bool {
        let subject = Self::normalize_search_text(&relation.subject);
        let object = Self::normalize_search_text(&relation.object);
        let predicate = Self::normalize_search_text(&relation.predicate);
        query_terms.iter().any(|term| {
            subject.contains(term.as_str())
                || object.contains(term.as_str())
                || predicate.contains(term.as_str())
        })
    }

    fn relation_identity(relation: &GraphRelationRecord) -> String {
        format!(
            "{}|{}|{}|{}",
            relation.namespace.as_deref().unwrap_or("global"),
            relation.subject,
            relation.predicate,
            relation.object
        )
    }

    fn relation_order_value(relation: &GraphRelationRecord) -> i64 {
        relation
            .order_index
            .unwrap_or_else(|| relation.updated_at.round() as i64)
    }

    fn document_priority_signal(
        category: &str,
        priority: &str,
        tags: &[String],
        metadata: &serde_json::Value,
    ) -> f64 {
        let mut score: f64 = 0.25;
        if matches!(category, "core" | "conversation") {
            score += 0.25;
        }
        if matches!(priority, "high" | "critical") {
            score += 0.20;
        }
        if tags.iter().any(|tag| {
            matches!(
                tag.as_str(),
                "decision" | "preference" | "owner" | "durable" | "profile"
            )
        }) {
            score += 0.20;
        }
        if metadata
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .map(|kind| matches!(kind, "decision" | "preference" | "profile"))
            .unwrap_or(false)
        {
            score += 0.10;
        }
        score.clamp(0.0, 1.0)
    }

    fn kv_priority_signal(key: &str, value: &serde_json::Value) -> f64 {
        let key_norm = Self::normalize_search_text(key);
        let value_norm = Self::normalize_search_text(&Self::render_kv_value(value));
        let mut score: f64 = 0.30;
        if ["preference", "decision", "profile", "setting", "owner"]
            .iter()
            .any(|needle| key_norm.contains(needle) || value_norm.contains(needle))
        {
            score += 0.35;
        }
        if value.is_object() || value.is_array() {
            score += 0.15;
        }
        score.clamp(0.0, 1.0)
    }

    fn render_kv_value(value: &serde_json::Value) -> String {
        match value {
            serde_json::Value::String(text) => text.clone(),
            _ => serde_json::to_string(value).unwrap_or_else(|_| value.to_string()),
        }
    }

    fn entity_label_with_type(name: &str, attrs: &serde_json::Value, role: &str) -> String {
        let entity_type = attrs
            .get("entity_types")
            .and_then(|et| et.get(role))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());
        match entity_type {
            Some(t) => format!("{name} ({t})"),
            None => name.to_string(),
        }
    }

    fn format_context_text(hits: &[NamespaceMemoryHit], query: Option<&str>) -> String {
        let mut parts = Vec::new();
        if let Some(query) = query {
            parts.push(format!("Query: {query}"));
        }
        for hit in hits {
            let summary = match hit.kind {
                MemoryItemKind::Document => {
                    let title = hit.title.clone().unwrap_or_else(|| hit.key.clone());
                    format!("{title}: {}", hit.content.trim())
                }
                MemoryItemKind::Kv => format!("[kv:{}] {}", hit.key, hit.content.trim()),
                MemoryItemKind::Episodic => {
                    format!("[episodic:{}] {}", hit.key, hit.content.trim())
                }
                MemoryItemKind::Event => {
                    format!("[event:{}] {}", hit.key, hit.content.trim())
                }
            };
            parts.push(summary);

            if !hit.supporting_relations.is_empty() {
                let relations = hit
                    .supporting_relations
                    .iter()
                    .map(|relation| {
                        let subject_label = Self::entity_label_with_type(
                            &relation.subject,
                            &relation.attrs,
                            "subject",
                        );
                        let object_label = Self::entity_label_with_type(
                            &relation.object,
                            &relation.attrs,
                            "object",
                        );
                        format!(
                            "{} -[{}]-> {}",
                            subject_label, relation.predicate, object_label
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                parts.push(format!("Relations: {relations}"));
            }
        }
        parts.join("\n\n")
    }
}

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;
