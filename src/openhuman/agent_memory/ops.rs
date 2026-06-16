//! Memory agent operations — benchmarking harness for memory tree walking
//! and retrieval performance measurement.

use crate::openhuman::agent_memory::types::{BenchmarkSummary, RetrievalStep, WalkBenchmark};
use crate::openhuman::config::Config;
use crate::openhuman::inference::provider::traits::Provider;
use crate::openhuman::memory::query::smart_walk::{
    run_smart_walk, SmartWalkOptions, SmartWalkStopReason,
};
use std::path::PathBuf;
use std::time::Instant;

/// Run a single benchmarked smart walk against the memory tree.
pub async fn bench_walk(
    config: &Config,
    provider: &dyn Provider,
    query: &str,
    namespace: &str,
    content_root: Option<PathBuf>,
    max_turns: usize,
    model: Option<String>,
) -> anyhow::Result<WalkBenchmark> {
    let effective_root = content_root
        .clone()
        .unwrap_or_else(|| config.memory_tree_content_root());

    log::info!(
        "[agent_memory::bench] query_len={} namespace={} content_root={} max_turns={}",
        query.len(),
        namespace,
        effective_root.display(),
        max_turns
    );

    let opts = SmartWalkOptions {
        max_turns,
        namespace: namespace.to_string(),
        model,
        content_root: content_root.clone(),
    };

    let start = Instant::now();
    let outcome = run_smart_walk(config, provider, query, opts).await?;
    let total_elapsed = start.elapsed();

    let steps: Vec<RetrievalStep> = outcome
        .trace
        .iter()
        .map(|step| RetrievalStep {
            turn: step.turn,
            action: step.action.clone(),
            args_summary: step.args_summary.clone(),
            result_preview: step.result_preview.clone(),
            elapsed: std::time::Duration::ZERO, // per-step timing not available from smart_walk yet
            chunks_returned: 0,
            bytes_scanned: 0,
        })
        .collect();

    let total_chunks = outcome.evidence.len();

    let stop_reason = match &outcome.stopped_reason {
        SmartWalkStopReason::Answered => "answered".to_string(),
        SmartWalkStopReason::MaxTurnsReached => "max_turns_reached".to_string(),
        SmartWalkStopReason::LlmGaveUp => "llm_gave_up".to_string(),
        SmartWalkStopReason::Error(e) => format!("error: {e}"),
    };

    let benchmark = WalkBenchmark {
        query: query.to_string(),
        namespace: namespace.to_string(),
        content_root: effective_root.display().to_string(),
        total_elapsed,
        steps,
        total_turns: outcome.turns_used,
        total_chunks_retrieved: total_chunks,
        total_bytes_scanned: outcome
            .evidence
            .iter()
            .map(|e| e.snippet.len() as u64)
            .sum(),
        answer: outcome.answer,
        stop_reason,
    };

    log::info!(
        "[agent_memory::bench] completed query_len={} elapsed={:?} turns={} chunks={} stop={}",
        query.len(),
        total_elapsed,
        benchmark.total_turns,
        benchmark.total_chunks_retrieved,
        benchmark.stop_reason
    );

    Ok(benchmark)
}

/// Run a batch of queries and produce a summary.
pub async fn bench_batch(
    config: &Config,
    provider: &dyn Provider,
    queries: &[&str],
    namespace: &str,
    content_root: Option<PathBuf>,
    max_turns: usize,
    model: Option<String>,
) -> anyhow::Result<(Vec<WalkBenchmark>, BenchmarkSummary)> {
    let mut results = Vec::with_capacity(queries.len());

    for query in queries {
        match bench_walk(
            config,
            provider,
            query,
            namespace,
            content_root.clone(),
            max_turns,
            model.clone(),
        )
        .await
        {
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
