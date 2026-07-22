//! Cache hit/miss + invalidation tests for the transcript view cache.

use super::TranscriptViewCache;
use crate::openhuman::agent::harness::session::transcript;
use std::path::Path;
use tempfile::TempDir;

fn meta_line(thread_id: &str) -> String {
    format!(
        r#"{{"_meta":{{"version":1,"agent":"orchestrator","dispatcher":"native","created":"2026-07-21T00:00:00Z","updated":"2026-07-21T00:00:00Z","turn_count":1,"input_tokens":0,"output_tokens":0,"cached_input_tokens":0,"charged_amount_usd":0.0,"thread_id":"{thread_id}"}}}}"#
    )
}

fn write_raw(workspace: &Path, stem: &str, thread_id: &str, body: &[&str]) -> std::path::PathBuf {
    let path = transcript::resolve_keyed_transcript_path(workspace, stem).unwrap();
    let mut buf = meta_line(thread_id);
    buf.push('\n');
    for line in body {
        buf.push_str(line);
        buf.push('\n');
    }
    std::fs::write(&path, buf).unwrap();
    path
}

#[test]
fn recomputes_when_file_grows() {
    let dir = TempDir::new().unwrap();
    let path = write_raw(
        dir.path(),
        "100_orchestrator",
        "thr_cache",
        &[r#"{"role":"user","content":"first"}"#],
    );
    let cache = TranscriptViewCache::default();

    let a = cache
        .get_or_project(dir.path(), "thr_cache")
        .expect("first");
    assert_eq!(a.items.len(), 1);

    // Second call, unchanged file → same cached Arc (hit).
    let b = cache
        .get_or_project(dir.path(), "thr_cache")
        .expect("second");
    assert!(
        std::sync::Arc::ptr_eq(&a, &b),
        "unchanged file must serve cached Arc"
    );

    // Append a line → file length changes → signature invalidates → recompute.
    let mut appended = std::fs::read_to_string(&path).unwrap();
    appended.push_str(r#"{"role":"assistant","content":"second"}"#);
    appended.push('\n');
    std::fs::write(&path, appended).unwrap();

    let c = cache
        .get_or_project(dir.path(), "thr_cache")
        .expect("third");
    assert!(!std::sync::Arc::ptr_eq(&a, &c), "grown file must recompute");
    assert_eq!(
        c.items.len(),
        2,
        "recomputed projection reflects the append"
    );
}

#[test]
fn missing_thread_returns_none() {
    let dir = TempDir::new().unwrap();
    let cache = TranscriptViewCache::default();
    assert!(cache.get_or_project(dir.path(), "ghost").is_none());
    assert_eq!(cache.len(), 0);
}
