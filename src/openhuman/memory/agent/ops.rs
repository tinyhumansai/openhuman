//! Memory agent operations — benchmarking harness for memory retrieval
//! performance measurement.
//!
//! Now measures the deterministic [`fast_retrieve`] retriever (E2GraphRAG).
//! There is no LLM in the loop, so the trace is a single retrieval "step"
//! rather than a multi-turn walk; `total_turns` stays 0 and the benchmark
//! focuses on wall-clock latency + hit count.

use crate::openhuman::config::Config;
use crate::openhuman::memory::agent::types::{BenchmarkSummary, RetrievalStep, WalkBenchmark};
use crate::openhuman::memory::api::provider::retrieval::{FastRetrieveQuery, RetrievalResponse};
use crate::openhuman::memory::source_scope::as_bus_scope;
use std::path::PathBuf;
use std::time::Instant;

/// Run a single benchmarked deterministic retrieval against the memory tree.
pub async fn bench_walk(
    config: &Config,
    query: &str,
    namespace: &str,
    content_root: Option<PathBuf>,
    limit: usize,
) -> anyhow::Result<WalkBenchmark> {
    let effective_root = content_root
        .clone()
        .unwrap_or_else(|| config.memory_tree_content_root());

    log::info!(
        "[agent_memory::bench] query_len={} namespace={} content_root={} limit={}",
        query.len(),
        namespace,
        effective_root.display(),
        limit
    );

    let opts = FastRetrieveQuery {
        limit,
        ..FastRetrieveQuery::default()
    };

    // Through the bound driver's `MemoryRetrieval`, not the engine (#5560).
    // `as_bus_scope()` renders this host's own source allowlist in the
    // contract's vocabulary — `binding.provider()` is the unguarded driver, so
    // the scope passed here IS the gate, and a literal `None` would fail it
    // open. See `memory::tree::retrieval::rpc`'s module docs.
    let scope = as_bus_scope();
    let binding = crate::openhuman::memory::binding::for_config(config)
        .map_err(|e| anyhow::anyhow!("bench_walk: bind memory driver: {e}"))?;
    let start = Instant::now();
    let resp = match binding.provider().as_retrieval() {
        Some(retrieval) => retrieval
            .fast_retrieve(query, opts, scope.as_ref())
            .await
            .map_err(|e| anyhow::anyhow!("bench_walk: fast_retrieve: {e}"))?,
        // A driver with no retrieval family keeps no summary tree to rank, so
        // an empty result is the honest benchmark rather than an error.
        None => {
            log::debug!(
                "[agent_memory::bench] driver '{}' does not serve Retrieval; reporting empty",
                binding.driver_id()
            );
            RetrievalResponse::default()
        }
    };
    let total_elapsed = start.elapsed();

    let total_bytes_scanned: u64 = resp.hits.iter().map(|h| h.content.len() as u64).sum();
    let steps: Vec<RetrievalStep> = vec![RetrievalStep {
        turn: 1,
        action: "fast_retrieve".to_string(),
        args_summary: format!("limit={limit}"),
        result_preview: format!("{} hits (total {})", resp.hits.len(), resp.total),
        elapsed: total_elapsed,
        chunks_returned: resp.hits.len(),
        bytes_scanned: total_bytes_scanned,
    }];

    let benchmark = WalkBenchmark {
        query: query.to_string(),
        namespace: namespace.to_string(),
        content_root: effective_root.display().to_string(),
        total_elapsed,
        steps,
        total_turns: 0, // deterministic — no LLM turns
        total_chunks_retrieved: resp.hits.len(),
        total_bytes_scanned,
        answer: String::new(), // synthesis is the high-level agent's job
        stop_reason: "deterministic".to_string(),
    };

    log::info!(
        "[agent_memory::bench] completed query_len={} elapsed={:?} chunks={}",
        query.len(),
        total_elapsed,
        benchmark.total_chunks_retrieved,
    );

    Ok(benchmark)
}

/// Run a batch of queries and produce a summary.
pub async fn bench_batch(
    config: &Config,
    queries: &[&str],
    namespace: &str,
    content_root: Option<PathBuf>,
    limit: usize,
) -> anyhow::Result<(Vec<WalkBenchmark>, BenchmarkSummary)> {
    let mut results = Vec::with_capacity(queries.len());

    for query in queries {
        match bench_walk(config, query, namespace, content_root.clone(), limit).await {
            Ok(bench) => results.push(bench),
            Err(e) => {
                log::warn!(
                    "[agent_memory::bench_batch] query={:?} failed: {e:#}",
                    query
                );
            }
        }
    }

    if results.is_empty() && !queries.is_empty() {
        anyhow::bail!(
            "[agent_memory::bench_batch] all {} queries failed",
            queries.len()
        );
    }

    let summary = BenchmarkSummary::from_benchmarks(&results);
    Ok((results, summary))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
