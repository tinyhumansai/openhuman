//! Memory agent operations — benchmarking harness for memory retrieval
//! performance measurement.
//!
//! Now measures the deterministic [`fast_retrieve`] retriever (E2GraphRAG).
//! There is no LLM in the loop, so the trace is a single retrieval "step"
//! rather than a multi-turn walk; `total_turns` stays 0 and the benchmark
//! focuses on wall-clock latency + hit count.

use crate::openhuman::agent_memory::types::{BenchmarkSummary, RetrievalStep, WalkBenchmark};
use crate::openhuman::config::Config;
use crate::openhuman::memory_tree::retrieval::{fast_retrieve, FastRetrieveOptions};
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

    let opts = FastRetrieveOptions {
        limit,
        ..FastRetrieveOptions::default()
    };

    let start = Instant::now();
    let resp = fast_retrieve(config, query, opts).await?;
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
mod tests {
    use super::*;

    #[test]
    fn empty_summary_is_zeroed() {
        let summary = BenchmarkSummary::from_benchmarks(&[]);
        assert_eq!(summary.runs, 0);
        assert_eq!(summary.avg_elapsed_ms, 0.0);
    }

    #[test]
    fn summary_from_single_run() {
        let bench = WalkBenchmark {
            query: "test".into(),
            namespace: "default".into(),
            content_root: "/tmp".into(),
            total_elapsed: std::time::Duration::from_millis(500),
            steps: vec![],
            total_turns: 3,
            total_chunks_retrieved: 5,
            total_bytes_scanned: 1024,
            answer: "test answer".into(),
            stop_reason: "answered".into(),
        };
        let summary = BenchmarkSummary::from_benchmarks(&[bench]);
        assert_eq!(summary.runs, 1);
        assert!((summary.avg_elapsed_ms - 500.0).abs() < 1.0);
        assert!((summary.avg_turns - 3.0).abs() < 0.01);
    }
}
