//! Integration coverage for on-disk memory artifacts.
//!
//! Uses a real Slack ingest to create raw/source artifacts, then stages a
//! mocked summary record to verify the Obsidian-compatible vault layout and
//! frontmatter contract without requiring a live summarizer model.

use tempfile::tempdir;

use chrono::{TimeZone, Utc};

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::memory_queue::drain_until_idle;
use openhuman_core::openhuman::memory_store::content::atomic::stage_summary;
use openhuman_core::openhuman::memory_store::content::obsidian::ensure_obsidian_defaults;
use openhuman_core::openhuman::memory_store::content::{SummaryComposeInput, SummaryTreeKind};
use openhuman_core::openhuman::memory_sync::composio::providers::slack::ingest::ingest_page_into_memory_tree;
use openhuman_core::openhuman::memory_sync::composio::providers::slack::SlackMessage;

fn make_config(workspace_dir: &std::path::Path) -> Config {
    let mut config = Config::default();
    config.workspace_dir = workspace_dir.to_path_buf();
    config
}

#[tokio::test]
async fn sync_raw_artifacts_and_mocked_summary_match_obsidian_contract() {
    let tmp = tempdir().expect("tempdir");
    let workspace_dir = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace_dir).expect("workspace dir");
    let config = make_config(&workspace_dir);

    let ts = Utc.timestamp_opt(1_700_000_000, 0).single().unwrap();
    let msg = SlackMessage {
        channel_id: "C123".into(),
        channel_name: "engineering".into(),
        is_private: false,
        author: "alice".into(),
        author_id: "U123".into(),
        text: "Phoenix migration launch window is Friday at 22:00 UTC.".into(),
        timestamp: ts,
        ts_raw: "1700000000.000100".into(),
        thread_ts: None,
        permalink: Some("https://slack.example.test/archives/C123/p1700000000000100".into()),
    };
    ingest_page_into_memory_tree(&config, "alice", "conn-slack-1", &[msg])
        .await
        .expect("seed slack sync");
    drain_until_idle(&config).await.expect("drain sync jobs");

    let content_root = config.memory_tree_content_root();
    let raw_file = content_root
        .join("raw")
        .join("slack-conn-slack-1")
        .join("chats")
        .join("1700000000000_1700000000.000100.md");
    let source_file = content_root
        .join("raw")
        .join("slack-conn-slack-1")
        .join("_source.md");
    assert!(raw_file.exists(), "expected raw markdown artifact");
    assert!(source_file.exists(), "expected source registry mirror");

    let raw_body = std::fs::read_to_string(&raw_file).expect("read raw artifact");
    assert!(raw_body.contains("**Channel:** #engineering"));
    assert!(raw_body.contains("**Author:** alice"));
    assert!(raw_body.contains("Phoenix migration launch window"));

    ensure_obsidian_defaults(&content_root).expect("stage obsidian defaults");

    let sealed_at = Utc.with_ymd_and_hms(2026, 5, 24, 22, 0, 0).unwrap();
    let child_ids = vec!["chunk-1".to_string()];
    let child_basenames = vec![Some("1700000000000_1700000000.000100".to_string())];
    let staged = stage_summary(
        &content_root,
        &SummaryComposeInput {
            summary_id: "summary:1760000000000:L1-phoenix-window",
            tree_kind: SummaryTreeKind::Source,
            tree_id: "source:slack",
            tree_scope: "slack:conn-slack-1",
            level: 1,
            child_ids: &child_ids,
            child_basenames: Some(&child_basenames),
            child_count: 1,
            time_range_start: ts,
            time_range_end: ts,
            sealed_at,
            body: "Phoenix migration launch window confirmed for Friday 22:00 UTC.",
        },
        "slack-conn-slack-1",
        None,
    )
    .expect("stage mocked summary");

    let summary_path = content_root.join(&staged.content_path);
    assert!(summary_path.exists(), "summary markdown should be written");
    let summary_body = std::fs::read_to_string(&summary_path).expect("read summary markdown");
    assert!(summary_body.contains("tree_kind: source"));
    assert!(summary_body.contains("tree_scope: \"slack:conn-slack-1\""));
    assert!(summary_body.contains("time_range_start: 2023-11-14T22:13:20+00:00"));
    assert!(summary_body.contains("time_range_end: 2023-11-14T22:13:20+00:00"));
    assert!(summary_body.contains("sealed_at: 2026-05-24T22:00:00+00:00"));
    assert!(summary_body.contains("[[1700000000000_1700000000.000100]]"));

    let graph_json = content_root.join(".obsidian").join("graph.json");
    let types_json = content_root.join(".obsidian").join("types.json");
    assert!(graph_json.exists(), "graph defaults should exist");
    assert!(types_json.exists(), "type hints should exist");
    let types_body = std::fs::read_to_string(types_json).expect("read types.json");
    assert!(types_body.contains("\"time_range_start\": \"date\""));
    assert!(types_body.contains("\"sealed_at\": \"datetime\""));
}
