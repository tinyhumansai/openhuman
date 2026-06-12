//! Types for memory agent retrieval performance tracking.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// A single retrieval operation performed during a memory walk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalStep {
    pub turn: usize,
    pub action: String,
    pub args_summary: String,
    pub result_preview: String,
    pub elapsed: Duration,
    pub chunks_returned: usize,
    pub bytes_scanned: u64,
}

/// Outcome of a benchmarked memory walk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkBenchmark {
    pub query: String,
    pub namespace: String,
    pub content_root: String,
    pub total_elapsed: Duration,
    pub steps: Vec<RetrievalStep>,
    pub total_turns: usize,
    pub total_chunks_retrieved: usize,
    pub total_bytes_scanned: u64,
    pub answer: String,
    pub stop_reason: String,
}

/// Summary statistics for a batch of benchmark runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkSummary {
    pub runs: usize,
    pub avg_elapsed_ms: f64,
    pub p50_elapsed_ms: f64,
    pub p95_elapsed_ms: f64,
    pub avg_turns: f64,
    pub avg_chunks: f64,
    pub total_bytes_scanned: u64,
}

impl BenchmarkSummary {
    pub fn from_benchmarks(benchmarks: &[WalkBenchmark]) -> Self {
        if benchmarks.is_empty() {
            return Self {
                runs: 0,
                avg_elapsed_ms: 0.0,
                p50_elapsed_ms: 0.0,
                p95_elapsed_ms: 0.0,
                avg_turns: 0.0,
                avg_chunks: 0.0,
                total_bytes_scanned: 0,
            };
        }

        let n = benchmarks.len() as f64;
        let mut elapsed_ms: Vec<f64> = benchmarks
            .iter()
            .map(|b| b.total_elapsed.as_secs_f64() * 1000.0)
            .collect();
        elapsed_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let avg_elapsed_ms = elapsed_ms.iter().sum::<f64>() / n;
        let p50_idx =
            ((benchmarks.len() as f64 * 0.5) as usize).min(benchmarks.len().saturating_sub(1));
        let p95_idx = ((benchmarks.len() as f64 * 0.95) as usize).min(benchmarks.len() - 1);

        Self {
            runs: benchmarks.len(),
            avg_elapsed_ms,
            p50_elapsed_ms: elapsed_ms[p50_idx],
            p95_elapsed_ms: elapsed_ms[p95_idx],
            avg_turns: benchmarks.iter().map(|b| b.total_turns as f64).sum::<f64>() / n,
            avg_chunks: benchmarks
                .iter()
                .map(|b| b.total_chunks_retrieved as f64)
                .sum::<f64>()
                / n,
            total_bytes_scanned: benchmarks.iter().map(|b| b.total_bytes_scanned).sum(),
        }
    }
}
