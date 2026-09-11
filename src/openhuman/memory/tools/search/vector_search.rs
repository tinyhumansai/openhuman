//! `memory_vector_search` — direct semantic search over chunk embeddings.
//!
//! Pure cosine similarity over stored chunk embeddings. No graph scoring,
//! no LLM loop. Fast, single embedding call. Supports metadata filtering,
//! cross-namespace search, similarity threshold, and MMR diversity.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::fmt::Write;

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::inference::embeddings::provider_from_config;
use crate::openhuman::memory::api::chunks::SourceKind;
use crate::openhuman::memory::api::provider::ChunkQuery;
use crate::openhuman::memory::api::provider::MemoryProvider;
use crate::openhuman::memory::ops::guard::active_memory_guard;
use crate::openhuman::tools::traits::{Tool, ToolResult};

pub struct MemoryVectorSearchTool;

// ── Ranking maths, brought home from the engine crate (#5560) ────────────────
//
// `cosine_similarity` and the MMR selector below were reached at
// `tinycortex::memory::{store::vectors, retrieval::mmr}`. They are ported here
// verbatim rather than routed at the module contract because there is nothing
// on the contract to route them *at*, and nothing that would benefit if there
// were: both are pure arithmetic over `&[f32]` slices this host already holds
// in memory. The chunks and their embeddings come back over the bus
// (`MemoryChunks::list_chunks` + `chunk_embeddings`); what happens to the
// numbers afterwards is this tool's ranking policy, and shipping vectors back
// across a bus boundary to have someone else multiply them would be a round
// trip bought for nothing.
//
// That is also why they are private to this file rather than a shared module:
// this tool is the only caller in the host. `archivist::boundary` keeps its own
// `f32` cosine for its own threshold — see the divergence note on
// [`cosine_similarity`], which is the reason those two are deliberately not
// unified.

/// Cosine similarity between two vectors, in `[-1.0, 1.0]`.
///
/// Returns `0.0` for mismatched lengths, empty vectors, or a zero-magnitude
/// vector on either side. Accumulates in `f64` regardless of the `f32` inputs,
/// so a long vector does not lose precision in the dot product.
///
/// # The `[0.0, 1.0]` clamp this does *not* do
///
/// The engine's `retrieval::mmr` module docs claim this function "clamps its
/// result to `[0.0, 1.0]`", which would make an anti-correlated candidate
/// indistinguishable from an orthogonal one inside [`mmr_select`]. **That
/// comment is stale**: the code clamps to `[-1.0, 1.0]` — the mathematical
/// range — and only to absorb floating-point drift. The port follows the code,
/// so a negatively-correlated candidate keeps its sign and is treated by MMR as
/// *more* diverse than an orthogonal one, which is the behaviour this tool has
/// actually had. Do not "restore" the clamp the stale comment describes.
///
/// Distinct from `archivist::boundary`'s private `cosine_similarity`, which is
/// `f32`-valued and unclamped; that one feeds a segment-boundary threshold, not
/// a ranking, and the two are independent on purpose.
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
    (dot / denom).clamp(-1.0, 1.0)
}

/// A candidate for MMR selection.
struct MmrCandidate<'a> {
    /// Caller-side index, echoed back on the result so the candidate can be
    /// resolved to its original record.
    index: usize,
    /// Candidate embedding; must share dimensionality with every other
    /// candidate, since cosine similarity is computed pairwise.
    embedding: &'a [f32],
    /// Precomputed relevance of this candidate to the query (here, its cosine
    /// score). Higher is more relevant; weighted by `lambda` in the MMR
    /// formula.
    relevance: f64,
}

/// Result of MMR selection: the original index and its MMR score.
struct MmrResult {
    /// Caller-side index echoed from the chosen [`MmrCandidate::index`], used
    /// to resolve the result back to its original record.
    index: usize,
    /// The MMR score at the step this item was selected:
    /// `lambda · relevance − (1 − lambda) · max_similarity(c, selected)`.
    /// Not comparable across runs with different `lambda`.
    ///
    /// Unread by this tool — the output reports each hit's *cosine* score, not
    /// its MMR score, because the latter depends on selection order and would
    /// read as an unstable percentage. Kept so the port stays a faithful copy
    /// of the engine's shape rather than a narrowed re-derivation.
    #[allow(
        dead_code,
        reason = "faithful port; this tool reports the cosine score instead"
    )]
    score: f64,
}

/// Select up to `limit` items from `candidates` using Maximal Marginal
/// Relevance, balancing relevance against redundancy within the selected set.
///
/// `lambda` controls the relevance-diversity tradeoff:
/// - `1.0` = pure relevance (no diversity)
/// - `0.0` = pure diversity (ignores relevance)
/// - `0.7` = the value this tool passes
///
/// For each selection step:
/// `mmr(c) = lambda · relevance(c) − (1 − lambda) · max_similarity(c, selected)`.
///
/// `lambda` is clamped to `[0.0, 1.0]`; `limit` is clamped to
/// `candidates.len()`. Returns `Vec::new()` immediately if `candidates` is
/// empty or `limit == 0`.
///
/// # Relevance is precomputed, not derived from a query vector here
///
/// The engine's signature took a `query_vec` first argument and never read it —
/// relevance came entirely from [`MmrCandidate::relevance`], which the caller
/// had already derived from the query with the same [`cosine_similarity`] used
/// below. The parameter is dropped in this port because the one call site fills
/// `relevance` exactly that way, so it was provably inert; the selection is
/// bit-for-bit what it was. If a future revision wants query-aware scoring
/// inside the loop, add the parameter back *and wire it*, rather than
/// reinstating a placeholder.
fn mmr_select(candidates: &[MmrCandidate<'_>], limit: usize, lambda: f64) -> Vec<MmrResult> {
    if candidates.is_empty() || limit == 0 {
        return Vec::new();
    }

    let lambda = lambda.clamp(0.0, 1.0);
    let limit = limit.min(candidates.len());

    let mut selected_embeddings: Vec<&[f32]> = Vec::with_capacity(limit);
    let mut results: Vec<MmrResult> = Vec::with_capacity(limit);
    let mut available: Vec<bool> = vec![true; candidates.len()];

    for _ in 0..limit {
        let mut best_idx: Option<usize> = None;
        let mut best_mmr = f64::NEG_INFINITY;

        for (i, candidate) in candidates.iter().enumerate() {
            if !available[i] {
                continue;
            }
            let max_sim_to_selected = if selected_embeddings.is_empty() {
                0.0
            } else {
                // Seeded with NEG_INFINITY, not 0.0: when every selected
                // similarity is negative, a 0.0 seed would win the fold and
                // report an anti-correlated candidate as orthogonal — the
                // exact collapse the `[0.0, 1.0]` note on
                // [`cosine_similarity`] warns against reintroducing. The
                // iterator is non-empty on this branch, so the seed never
                // escapes.
                selected_embeddings
                    .iter()
                    .map(|sel| cosine_similarity(candidate.embedding, sel))
                    .fold(f64::NEG_INFINITY, f64::max)
            };
            let mmr_score = lambda * candidate.relevance - (1.0 - lambda) * max_sim_to_selected;
            if mmr_score > best_mmr {
                best_mmr = mmr_score;
                best_idx = Some(i);
            }
        }

        let Some(idx) = best_idx else { break };
        available[idx] = false;
        selected_embeddings.push(candidates[idx].embedding);
        results.push(MmrResult {
            index: candidates[idx].index,
            score: best_mmr,
        });
    }

    results
}

#[derive(Debug, Deserialize)]
struct Args {
    query: String,
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    source_kind: Option<String>,
    #[serde(default)]
    time_window_days: Option<u32>,
    #[serde(default)]
    min_score: Option<f64>,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    diverse: bool,
}

fn default_limit() -> usize {
    10
}

#[async_trait]
impl Tool for MemoryVectorSearchTool {
    fn name(&self) -> &str {
        "memory_vector_search"
    }

    fn description(&self) -> &str {
        "Direct semantic vector search over memory chunks. Embeds the query \
         and finds the most similar stored content by cosine similarity. \
         Fast (single embedding call, no LLM). Use for semantic lookup when \
         you know roughly what you're looking for. Returns chunk-level results \
         with scores."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural-language query to embed and search against stored memory chunks."
                },
                "source_kind": {
                    "type": "string",
                    "enum": ["chat", "email", "document"],
                    "description": "Filter to a specific source type."
                },
                "time_window_days": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Only include chunks from the last N days."
                },
                "min_score": {
                    "type": "number",
                    "minimum": 0.0,
                    "maximum": 1.0,
                    "description": "Minimum cosine similarity threshold (default 0.3)."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 50,
                    "description": "Max results to return (default 10)."
                },
                "diverse": {
                    "type": "boolean",
                    "description": "Apply MMR diversity to reduce redundancy among results (default false)."
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let parsed: Args = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("invalid arguments for memory_vector_search: {e}"))?;

        if parsed.query.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "memory_vector_search: query cannot be empty"
            ));
        }

        let limit = parsed.limit.clamp(1, 50);
        let min_score = parsed.min_score.unwrap_or(0.3);

        log::debug!(
            "[tool][memory_vector_search] query_len={} source_kind={:?} window={:?} min_score={} limit={} diverse={}",
            parsed.query.len(),
            parsed.source_kind,
            parsed.time_window_days,
            min_score,
            limit,
            parsed.diverse,
        );

        let config = config_rpc::load_config_with_timeout()
            .await
            .map_err(|e| anyhow::anyhow!("memory_vector_search: load config failed: {e}"))?;

        // Chunks are read through the bound driver, not by opening the store
        // in this process. Before the module port this called
        // `list_chunks(&config, …)` directly, which resolved the workspace path
        // and opened the same SQLite database the loaded module already had
        // open — two engine instances over one file, with the module not
        // authoritative. See `docs/specs/2026-08-13-memory-module-port.md` §2.1.
        let guard = active_memory_guard()
            .await
            .map_err(|e| anyhow::anyhow!("memory_vector_search: {e}"))?;
        let chunk_reader = guard.as_chunks().ok_or_else(|| {
            anyhow::anyhow!("memory_vector_search: memory driver does not support the chunk family")
        })?;

        let embedder = provider_from_config(&config)
            .map_err(|e| anyhow::anyhow!("memory_vector_search: embedding provider failed: {e}"))?;

        let query_vec = embedder
            .embed_one(&parsed.query)
            .await
            .map_err(|e| anyhow::anyhow!("memory_vector_search: embedding query failed: {e}"))?;

        let source_kind = match parsed.source_kind.as_deref() {
            Some(s) => Some(
                SourceKind::parse(s).map_err(|e| anyhow::anyhow!("memory_vector_search: {e}"))?,
            ),
            None => None,
        };

        let since_ms = parsed.time_window_days.map(|days| {
            let now_ms = chrono::Utc::now().timestamp_millis();
            now_ms - (i64::from(days) * 86_400_000)
        });

        // Fetch candidate chunks with metadata filters. The per-profile
        // memory-source gate is applied inside the driver's query (before the
        // row limit), so disallowed-source chunks can't starve permitted ones.
        //
        // `None` for the scope is not "unrestricted": the guard intersects it
        // with the ambient per-turn allowlist and passes the result down, so
        // naming a scope here could only ever *narrow* what the turn may see.
        let query = ChunkQuery {
            source_kind,
            source_id: None,
            owner: None,
            since_ms,
            until_ms: None,
            limit: Some(1000),
            offset: None,
            exclude_dropped: false,
            // The filtered-listing predicates this caller does not use. An
            // empty predicate is unfiltered, so the defaults leave the query
            // exactly as narrow as the fields above already make it.
            ..Default::default()
        };

        let chunks = chunk_reader
            .list_chunks(&query, None)
            .await
            .map_err(|e| anyhow::anyhow!("memory_vector_search: list chunks failed: {e}"))?;

        if chunks.is_empty() {
            return Ok(ToolResult::success("No chunks found matching filters."));
        }

        // Get embeddings for these chunks
        let chunk_ids: Vec<String> = chunks.iter().map(|c| c.id.clone()).collect();
        let model_sig = embedder.signature();
        let embeddings: std::collections::HashMap<String, Vec<f32>> = chunk_reader
            .chunk_embeddings(&chunk_ids, &model_sig)
            .await
            .map_err(|e| anyhow::anyhow!("memory_vector_search: load embeddings failed: {e}"))?
            .into_iter()
            .map(|embedding| (embedding.chunk_id, embedding.vector))
            .collect();

        // Score each chunk
        let mut scored: Vec<(usize, f64, &[f32])> = Vec::new();

        for (idx, chunk) in chunks.iter().enumerate() {
            let Some(emb) = embeddings.get(&chunk.id) else {
                continue;
            };
            if emb.len() != query_vec.len() {
                continue;
            }
            let score = cosine_similarity(&query_vec, emb);
            if score >= min_score {
                scored.push((idx, score, emb.as_slice()));
            }
        }

        if scored.is_empty() {
            return Ok(ToolResult::success(
                "No chunks scored above the similarity threshold.",
            ));
        }

        let results = if parsed.diverse && scored.len() > limit {
            let candidates: Vec<MmrCandidate<'_>> = scored
                .iter()
                .map(|(idx, score, emb)| MmrCandidate {
                    index: *idx,
                    embedding: emb,
                    relevance: *score,
                })
                .collect();
            let mmr_results = mmr_select(&candidates, limit, 0.7);
            mmr_results
                .into_iter()
                .map(|r| {
                    (
                        r.index,
                        scored.iter().find(|(i, _, _)| *i == r.index).unwrap().1,
                    )
                })
                .collect::<Vec<_>>()
        } else {
            scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            scored.truncate(limit);
            scored
                .iter()
                .map(|(idx, score, _)| (*idx, *score))
                .collect()
        };

        let mut output = format!("Found {} results:\n\n", results.len());
        for (chunk_idx, score) in &results {
            let chunk = &chunks[*chunk_idx];
            let preview: String = chunk.content.chars().take(300).collect();
            let truncated = if chunk.content.chars().count() > 300 {
                "..."
            } else {
                ""
            };
            let _ = writeln!(
                output,
                "- [{:.0}%] source={}:{} id={}\n  {}{}",
                score * 100.0,
                chunk.metadata.source_kind.as_str(),
                chunk.metadata.source_id,
                chunk.id,
                preview,
                truncated,
            );
        }

        log::debug!(
            "[tool][memory_vector_search] returning {} results from {} candidates",
            results.len(),
            chunks.len(),
        );

        Ok(ToolResult::success(output))
    }
}

#[cfg(test)]
#[path = "vector_search_tests.rs"]
mod tests;
