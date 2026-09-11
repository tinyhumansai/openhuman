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
