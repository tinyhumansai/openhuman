//! `memory_hybrid_search` — configurable multi-signal hybrid search.
//!
//! Exposes the existing hybrid retrieval engine (graph + vector + keyword +
//! freshness) with tunable weight profiles. The agent chooses a mode that
//! emphasizes the signal most relevant to its current need.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::fmt::Write;

use crate::openhuman::memory::api::provider::MemoryProvider;
use crate::openhuman::memory::api::types::MemoryItemKind;
use crate::openhuman::memory::ops::guard::active_memory_guard;
use crate::openhuman::tools::traits::{Tool, ToolResult};

pub struct MemoryHybridSearchTool;

// ── Weight profiles and the re-ranking sum, brought home (#5560) ─────────────
//
// `WeightProfile` was `tinycortex::memory::WeightProfile` and the fold below
// was `tinycortex::memory::retrieval::scoring::hybrid_score`. Both are ported
// here rather than routed at the module contract, for the same reason the
// vector tool's cosine is: they are pure arithmetic over four numbers the
// driver has *already sent*. `MemoryRetrieval::recall_namespace_scored`
// answers with each hit's `score_breakdown`, so the four raw signals are in
// hand; re-weighting them is this tool's ranking policy and needs no bus at
// all.
//
// This is the same split the engine already drew. Its own `scoring` module docs
// say the profiles "live in `memory::config` and are read from config — never
// hardcoded here", i.e. the weights were always the *caller's* input to a
// function that only multiplied and added. The `mode` argument on this tool is
// where that input comes from, so the table belongs beside it.

/// Named hybrid-retrieval weight profiles (graph / vector / keyword /
/// freshness), resolved from this tool's `mode` argument.
///
/// The final ranking score is the plain weighted sum `graph·graph_relevance +
/// vector·vector_similarity + keyword·keyword_relevance + freshness·freshness`.
/// Nothing here *enforces* that the four weights sum to
/// `1.0` — the four built-ins are chosen that way by convention so scores land
/// in a familiar `[0.0, 1.0]`-ish range when every signal is itself in
/// `[0.0, 1.0]`. The constants are the engine's, value for value, so a query
/// ranks exactly as it did before.
#[derive(Debug, Clone, Copy, PartialEq)]
struct WeightProfile {
    /// Weight on graph/co-occurrence proximity signal.
    graph: f64,
    /// Weight on dense vector (cosine) similarity signal.
    vector: f64,
    /// Weight on lexical/keyword match signal.
    keyword: f64,
    /// Weight on recency; `0.0` disables freshness boosting.
    freshness: f64,
}

impl WeightProfile {
    /// `balanced`: graph 0.35, vector 0.35, keyword 0.15, freshness 0.15.
    const BALANCED: Self = Self {
        graph: 0.35,
        vector: 0.35,
        keyword: 0.15,
        freshness: 0.15,
    };
    /// `semantic`: graph 0.15, vector 0.65, keyword 0.20.
    const SEMANTIC: Self = Self {
        graph: 0.15,
        vector: 0.65,
        keyword: 0.20,
        freshness: 0.0,
    };
    /// `lexical`: graph 0.25, vector 0.15, keyword 0.60.
    const LEXICAL: Self = Self {
        graph: 0.25,
        vector: 0.15,
        keyword: 0.60,
        freshness: 0.0,
    };
    /// `graph_first`: graph 0.55, vector 0.30, keyword 0.15.
    const GRAPH_FIRST: Self = Self {
        graph: 0.55,
        vector: 0.30,
        keyword: 0.15,
        freshness: 0.0,
    };

    /// Resolve a profile by its wire name, returning `None` for unknown names.
    ///
    /// The names are the `mode` enum in [`MemoryHybridSearchTool`]'s parameter
    /// schema and are therefore a published surface — a rename here is a
    /// breaking change to what the model may ask for, not a refactor.
    fn by_name(name: &str) -> Option<Self> {
        match name {
            "balanced" => Some(Self::BALANCED),
            "semantic" => Some(Self::SEMANTIC),
            "lexical" => Some(Self::LEXICAL),
            "graph_first" => Some(Self::GRAPH_FIRST),
            _ => None,
        }
    }
}

/// Fold four raw signals into one ranking score under `profile`.
///
/// Each signal is expected in `[0.0, 1.0]`; the result is the weighted sum
/// `graph·g + vector·v + keyword·k + freshness·f`.
///
/// The engine's `hybrid_score` returned a whole `RetrievalScoreBreakdown` and
/// this call site read `.final_score` off it and dropped the rest — the other
/// five fields were the caller's own inputs echoed back, plus a hardcoded
/// `episodic_relevance: 0.0` carried for wire compatibility with a payload
/// nothing here serialises. So this returns the number instead of rebuilding a
/// breakdown to immediately discard; the arithmetic is unchanged.
fn hybrid_final_score(
    profile: &WeightProfile,
    graph_relevance: f64,
    vector_similarity: f64,
    keyword_relevance: f64,
    freshness: f64,
) -> f64 {
    profile.graph * graph_relevance
        + profile.vector * vector_similarity
        + profile.keyword * keyword_relevance
        + profile.freshness * freshness
}

#[derive(Debug, Deserialize)]
struct Args {
    query: String,
    namespace: String,
    #[serde(default = "default_mode")]
    mode: String,
    #[serde(default = "default_limit")]
    limit: u32,
    #[serde(default)]
    include_breakdown: bool,
}

fn default_mode() -> String {
    "balanced".to_string()
}

fn default_limit() -> u32 {
    10
}

fn kind_label(kind: &MemoryItemKind) -> &'static str {
    match kind {
        MemoryItemKind::Document => "doc",
        MemoryItemKind::Kv => "kv",
        MemoryItemKind::Episodic => "episodic",
        MemoryItemKind::Event => "event",
    }
}

#[async_trait]
impl Tool for MemoryHybridSearchTool {
    fn name(&self) -> &str {
        "memory_hybrid_search"
    }

    fn description(&self) -> &str {
        "Multi-signal hybrid search with configurable weight profiles. \
         Combines graph relevance, vector similarity, keyword matching, \
         and freshness into a unified score. Choose a mode to emphasize \
         the signal most relevant to your query: 'balanced' (equal graph+vector), \
         'semantic' (vector-heavy), 'lexical' (keyword-heavy), \
         'graph_first' (relationship-heavy)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["query", "namespace"],
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural-language search query."
                },
                "namespace": {
                    "type": "string",
                    "description": "Namespace to search (e.g. 'global', 'background')."
                },
                "mode": {
                    "type": "string",
                    "enum": ["balanced", "semantic", "lexical", "graph_first"],
                    "description": "Weight profile: 'balanced' (default), 'semantic' (vector-heavy), 'lexical' (keyword-heavy), 'graph_first' (relationship-heavy)."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 50,
                    "description": "Max results (default 10)."
                },
                "include_breakdown": {
                    "type": "boolean",
                    "description": "Show per-signal score breakdown for each result (default false)."
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let parsed: Args = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("invalid arguments for memory_hybrid_search: {e}"))?;

        if parsed.query.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "memory_hybrid_search: query cannot be empty"
            ));
        }
        if parsed.namespace.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "memory_hybrid_search: namespace cannot be empty"
            ));
        }

        let profile = WeightProfile::by_name(&parsed.mode).ok_or_else(|| {
            log::warn!(
                "[tool][memory_hybrid_search] rejected unknown mode={}",
                parsed.mode
            );
            anyhow::anyhow!(
                "memory_hybrid_search: unknown mode '{}'; expected balanced, semantic, lexical, or graph_first",
                parsed.mode
            )
        })?;
        let limit = parsed.limit.clamp(1, 50);

        log::debug!(
            "[tool][memory_hybrid_search] query_len={} ns={} mode={} limit={}",
            parsed.query.len(),
            parsed.namespace,
            parsed.mode,
            limit,
        );

        // Reads through the bound driver. This used to call
        // `UnifiedMemory::new(&config.workspace_dir, …)` — constructing a
        // *whole second engine* over the workspace the loaded module already
        // owns, the most severe instance of the split brain this port removes.
        let guard = active_memory_guard()
            .await
            .map_err(|e| anyhow::anyhow!("memory_hybrid_search: {e}"))?;
        let retrieval = guard.as_retrieval().ok_or_else(|| {
            anyhow::anyhow!(
                "memory_hybrid_search: memory driver does not support the retrieval family"
            )
        })?;

        // Self-echo guard (agent-agnostic, mirrors `UnifiedMemory::recall`):
        // exclude documents auto-saved for the ambient chat thread (set by
        // the web channel around the turn) so a search issued mid-turn
        // never retrieves the very request that triggered it. `None`
        // outside a chat turn — unchanged behavior for cron/CLI/tests.
        let exclude_session_id =
            crate::openhuman::agent::tinyagents::thread_context::current_thread_id();
        if let Some(ref excluded) = exclude_session_id {
            log::debug!(
                "[tool][memory_hybrid_search] applying same-session exclusion exclude_session_id={excluded}"
            );
        }
        let hits = retrieval
            .recall_namespace_scored(
                &parsed.namespace,
                &parsed.query,
                limit as usize,
                exclude_session_id.as_deref(),
            )
            .await
            .map_err(|e| anyhow::anyhow!("memory_hybrid_search: query failed: {e}"))?;

        if hits.is_empty() {
            return Ok(ToolResult::success("No results found."));
        }

        // Re-score using the selected weight profile
        let mut rescored: Vec<(usize, f64)> = hits
            .iter()
            .enumerate()
            .map(|(i, hit)| {
                let bd = &hit.score_breakdown;
                let score = hybrid_final_score(
                    &profile,
                    bd.graph_relevance,
                    bd.vector_similarity,
                    bd.keyword_relevance,
                    bd.freshness,
                );
                (i, score)
            })
            .filter(|(_, score)| *score > 0.0)
            .collect();

        rescored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        rescored.truncate(limit as usize);

        let mut output = format!(
            "Found {} results (mode={}):\n\n",
            rescored.len(),
            parsed.mode,
        );

        for (hit_idx, score) in &rescored {
            let hit = &hits[*hit_idx];
            let preview: String = hit.content.chars().take(200).collect();
            let truncated = if hit.content.chars().count() > 200 {
                "..."
            } else {
                ""
            };
            let _ = writeln!(
                output,
                "- [{:.0}%] [{}] {}: {}{}",
                score * 100.0,
                kind_label(&hit.kind),
                hit.key,
                preview,
                truncated,
            );

            if parsed.include_breakdown {
                let bd = &hit.score_breakdown;
                let _ = writeln!(
                    output,
                    "  scores: graph={:.2} vector={:.2} keyword={:.2} freshness={:.2}",
                    bd.graph_relevance, bd.vector_similarity, bd.keyword_relevance, bd.freshness,
                );
            }
        }

        log::debug!(
            "[tool][memory_hybrid_search] returning {} results",
            rescored.len(),
        );

        Ok(ToolResult::success(output))
    }
}

#[cfg(test)]
#[path = "hybrid_search_tests.rs"]
mod tests;
