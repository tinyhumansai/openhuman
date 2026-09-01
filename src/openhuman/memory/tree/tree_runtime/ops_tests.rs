use super::*;
use chrono::TimeZone;
use tempfile::TempDir;

// `TreeNode`, `level_from_node_id` and `derive_parent_id` used to arrive
// through `super::*` while `ops.rs` still globbed the engine crate's runtime
// module. `ops.rs` names the contract explicitly now (#5560) and imports only
// the two items it uses, so these are named here — the same items, from the
// same crate the sibling `tree_runtime/mod.rs` re-exports them from.
use crate::openhuman::memory::api::tree::{derive_parent_id, level_from_node_id, TreeNode};

// The handlers under test resolve a `MemoryProvider` now, so these tests bind
// one. See `tree_runtime::test_support` for what it is and why it is backed by
// the real engine store rather than a fake.
use crate::openhuman::memory::tree::tree_runtime::test_support::{bind_tree_driver, engine_store};

fn rfc3339_z(ts: DateTime<Utc>) -> String {
    ts.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn config_in_tempdir() -> (TempDir, Config) {
    let tmp = TempDir::new().expect("tempdir");
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    (tmp, cfg)
}

fn test_node(
    namespace: &str,
    node_id: &str,
    summary: &str,
    created_at: DateTime<Utc>,
    child_count: u32,
) -> TreeNode {
    TreeNode {
        node_id: node_id.to_string(),
        namespace: namespace.to_string(),
        level: level_from_node_id(node_id),
        parent_id: derive_parent_id(node_id),
        summary: summary.to_string(),
        token_count: estimate_tokens(summary),
        child_count,
        created_at,
        updated_at: created_at,
        metadata: None,
    }
}

#[test]
fn create_provider_uses_local_model_when_local_ai_enabled() {
    // #002 FR-007: local path returns the user's local chat model.
    let mut cfg = Config::default();
    cfg.local_ai.runtime_enabled = true;
    cfg.local_ai.chat_model_id = "qwen2.5:7b".to_string();
    let (_provider, model) = create_provider(&cfg).expect("local provider should build");
    assert_eq!(model, "qwen2.5:7b");
}

#[test]
fn create_provider_errors_without_cloud_opt_in() {
    // By default, cloud summarization is off — memory summaries are
    // sensitive, so an explicit opt-in is required before routing them to
    // an external provider.
    let mut cfg = Config::default();
    cfg.local_ai.runtime_enabled = false;
    // cloud_summarization_opt_in defaults to false
    match create_provider(&cfg) {
        Err(e) => assert!(
            e.contains("no summarization provider"),
            "unexpected error: {e}"
        ),
        Ok(_) => panic!("expected error without cloud opt-in"),
    }
}

#[test]
fn create_provider_uses_cloud_when_opted_in_and_local_ai_off() {
    // #002 FR-007: with explicit opt-in Build Summary Trees uses the
    // configured cloud provider when local AI is disabled.
    let mut cfg = Config::default();
    cfg.local_ai.runtime_enabled = false;
    cfg.memory_tree.cloud_summarization_opt_in = true;
    let (_provider, model) =
        create_provider(&cfg).expect("cloud fallback should build when opted in");
    assert!(
        !model.trim().is_empty(),
        "cloud fallback must resolve a model"
    );
}

#[tokio::test]
async fn tree_summarizer_ingest_rejects_blank_content() {
    let (_tmp, cfg) = config_in_tempdir();
    bind_tree_driver(&cfg);
    let err = tree_summarizer_ingest(&cfg, "team", "   ", None, None)
        .await
        .expect_err("blank content should be rejected");
    assert!(err.contains("content must not be empty"));
}

#[tokio::test]
async fn tree_summarizer_ingest_writes_buffer_and_reports_metadata() {
    let (_tmp, cfg) = config_in_tempdir();
    let ts = chrono::Utc
        .with_ymd_and_hms(2026, 5, 24, 12, 30, 0)
        .unwrap();
    let meta = json!({"source": "unit-test"});
    bind_tree_driver(&cfg);
    let outcome =
        tree_summarizer_ingest(&cfg, "Team / Notes", "hello world", Some(ts), Some(&meta))
            .await
            .expect("ingest should succeed");

    assert_eq!(
        outcome.logs,
        vec!["content buffered for namespace 'Team / Notes'".to_string()]
    );
    assert_eq!(outcome.value["buffered"], true);
    assert_eq!(outcome.value["namespace"], "Team / Notes");
    assert_eq!(
        outcome.value["tokens"],
        json!(estimate_tokens("hello world"))
    );
    assert_eq!(outcome.value["has_metadata"], true);

    let path = outcome.value["path"]
        .as_str()
        .expect("path string in response");
    let written = std::fs::read_to_string(path).expect("buffer file should exist");
    assert!(written.contains("hello world"));
    assert!(written.contains("\"source\":\"unit-test\""));
}

#[tokio::test]
async fn tree_summarizer_status_reports_empty_tree_defaults() {
    let (_tmp, cfg) = config_in_tempdir();
    bind_tree_driver(&cfg);
    let outcome = tree_summarizer_status(&cfg, "fresh-ns")
        .await
        .expect("status on fresh namespace");
    assert_eq!(
        outcome.logs,
        vec!["tree status for namespace 'fresh-ns'".to_string()]
    );
    assert_eq!(outcome.value["namespace"], "fresh-ns");
    assert_eq!(outcome.value["total_nodes"], 0);
    assert_eq!(outcome.value["depth"], 0);
}

#[tokio::test]
async fn tree_summarizer_query_errors_when_node_is_missing() {
    let (_tmp, cfg) = config_in_tempdir();
    bind_tree_driver(&cfg);
    let err = tree_summarizer_query(&cfg, "fresh-ns", Some("root"))
        .await
        .expect_err("missing node should error");
    assert!(err.contains("node 'root' not found in namespace 'fresh-ns'"));
}

#[tokio::test]
async fn tree_summarizer_query_returns_node_and_children() {
    let (_tmp, cfg) = config_in_tempdir();
    let ts = chrono::Utc
        .with_ymd_and_hms(2026, 5, 24, 12, 30, 0)
        .unwrap();
    let root = test_node("team", "root", "root summary", ts, 1);
    let year = test_node("team", "2026", "year summary", ts, 1);
    engine_store::write_node(&cfg, &root).expect("write root");
    engine_store::write_node(&cfg, &year).expect("write year");
    bind_tree_driver(&cfg);

    let outcome = tree_summarizer_query(&cfg, "team", None)
        .await
        .expect("query should succeed");

    assert_eq!(
        outcome.logs,
        vec!["queried node 'root' in namespace 'team'"]
    );
    assert_eq!(outcome.value["node"]["node_id"], "root");
    assert_eq!(outcome.value["node"]["summary"], "root summary");
    assert_eq!(
        outcome.value["children"],
        json!([{
            "node_id": "2026",
            "namespace": "team",
            "level": "year",
            "parent_id": "root",
            "summary": "year summary",
            "token_count": estimate_tokens("year summary"),
            "child_count": 1,
            "created_at": rfc3339_z(ts),
            "updated_at": rfc3339_z(ts)
        }])
    );
}

#[tokio::test]
async fn tree_summarizer_status_reports_populated_tree_details() {
    let (_tmp, cfg) = config_in_tempdir();
    let early = chrono::Utc.with_ymd_and_hms(2026, 5, 24, 8, 0, 0).unwrap();
    let late = chrono::Utc.with_ymd_and_hms(2026, 5, 24, 17, 0, 0).unwrap();
    for node in [
        test_node("team", "root", "root summary", early, 1),
        test_node("team", "2026", "year summary", early, 1),
        test_node("team", "2026/05", "month summary", early, 1),
        test_node("team", "2026/05/24", "day summary", early, 2),
        test_node("team", "2026/05/24/08", "hour one", early, 0),
        test_node("team", "2026/05/24/17", "hour two", late, 0),
    ] {
        engine_store::write_node(&cfg, &node).expect("write test node");
    }
    bind_tree_driver(&cfg);

    let outcome = tree_summarizer_status(&cfg, "team")
        .await
        .expect("status should succeed");

    assert_eq!(outcome.logs, vec!["tree status for namespace 'team'"]);
    assert_eq!(outcome.value["namespace"], "team");
    assert_eq!(outcome.value["total_nodes"], 6);
    assert_eq!(outcome.value["depth"], 5);
    assert_eq!(outcome.value["oldest_entry"], rfc3339_z(early));
    assert_eq!(outcome.value["newest_entry"], rfc3339_z(late));
    assert_eq!(outcome.value["last_run_at"], Value::Null);
}

#[tokio::test]
async fn tree_summarizer_run_skips_when_buffer_is_empty() {
    let (_tmp, mut cfg) = config_in_tempdir();
    cfg.local_ai.runtime_enabled = true;
    bind_tree_driver(&cfg);

    let outcome = tree_summarizer_run(&cfg, "team")
        .await
        .expect("empty buffer should skip");

    assert_eq!(
        outcome.logs,
        vec!["summarization skipped for 'team': no buffered data"]
    );
    assert_eq!(
        outcome.value,
        json!({ "skipped": true, "reason": "no buffered data" })
    );
    assert!(
        !engine_store::buffer_dir(&cfg, "team").exists(),
        "skip path should not create a buffer directory"
    );
}

#[tokio::test]
async fn tree_summarizer_run_skips_cleanly_with_cloud_fallback_and_empty_buffer() {
    // #002 FR-007 (Gray review updated): with local AI off AND explicit cloud
    // opt-in, run/rebuild do not hard-error on the provider precondition.
    // With an empty buffer, `run` reports the normal "no buffered data" skip.
    let (_tmp, mut cfg) = config_in_tempdir();
    cfg.local_ai.runtime_enabled = false;
    cfg.memory_tree.cloud_summarization_opt_in = true;
    bind_tree_driver(&cfg);

    let outcome = tree_summarizer_run(&cfg, "team")
        .await
        .expect("run should not error on the provider precondition when opted in");
    assert_eq!(
        outcome.value,
        json!({ "skipped": true, "reason": "no buffered data" })
    );

    // Rebuild on an empty tree returns the (zero-node) status, not an error.
    let rebuilt = tree_summarizer_rebuild(&cfg, "team")
        .await
        .expect("rebuild should not error on the provider precondition when opted in");
    assert_eq!(rebuilt.value["total_nodes"], 0);
}
