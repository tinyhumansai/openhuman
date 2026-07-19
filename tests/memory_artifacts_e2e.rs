//! Integration coverage for on-disk memory artifacts.
//!
//! Uses a real Slack ingest to create raw/source artifacts, then stages a
//! mocked summary record to verify the Obsidian-compatible vault layout and
//! frontmatter contract without requiring a live summarizer model.

use tempfile::tempdir;

use chrono::{TimeZone, Utc};

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::memory::ingest_pipeline::ingest_chat;
use openhuman_core::openhuman::memory::tree_source::registry::get_or_create_source_tree;
use openhuman_core::openhuman::memory_queue::drain_until_idle;
use openhuman_core::openhuman::memory_store::content::atomic::stage_summary;
use openhuman_core::openhuman::memory_store::content::obsidian::ensure_obsidian_defaults;
use openhuman_core::openhuman::memory_store::content::raw::{write_raw_items, RawItem, RawKind};
use openhuman_core::openhuman::memory_store::content::wiki_git::{
    get_read_pointer_tag, set_read_pointer_tag,
};
use openhuman_core::openhuman::memory_store::content::{SummaryComposeInput, SummaryTreeKind};
use openhuman_core::openhuman::memory_sync::canonicalize::chat::{ChatBatch, ChatMessage};
use openhuman_core::openhuman::memory_tree::ingest::{ingest_summary, SummaryIngestInput};

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
    write_raw_items(
        &config.memory_tree_content_root(),
        "slack:conn-slack-1",
        &[RawItem {
            uid: "1700000000.000100",
            created_at_ms: ts.timestamp_millis(),
            markdown: "**Channel:** #engineering\n**Author:** alice\n\nPhoenix migration launch window is Friday at 22:00 UTC.",
            kind: RawKind::Chat,
        }],
    )
    .expect("seed raw Slack artifact");
    let batch = ChatBatch {
        platform: "slack".into(),
        channel_label: "#engineering".into(),
        messages: vec![ChatMessage {
            author: "alice".into(),
            timestamp: ts,
            text: "Phoenix migration launch window is Friday at 22:00 UTC.".into(),
            source_ref: Some("https://slack.example.test/archives/C123/p1700000000000100".into()),
        }],
    };
    ingest_chat(
        &config,
        "slack:conn-slack-1",
        "alice",
        vec!["slack".into(), "ingested".into()],
        batch,
    )
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

#[tokio::test]
async fn summary_ingest_records_summary_only_git_history_and_timestamped_read_tags() {
    let tmp = tempdir().expect("tempdir");
    let workspace_dir = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace_dir).expect("workspace dir");
    let config = make_config(&workspace_dir);
    let content_root = config.memory_tree_content_root();
    let raw_path = content_root.join("wiki/raw/not-tracked.md");
    std::fs::create_dir_all(raw_path.parent().unwrap()).expect("raw dir");
    std::fs::write(&raw_path, "should stay out of git history").expect("seed raw file");

    let tree = get_or_create_source_tree(&config, "github:tinyhumansai/openhuman")
        .expect("create source tree");
    let start = Utc.with_ymd_and_hms(2026, 6, 26, 9, 0, 0).unwrap();
    let end = Utc.with_ymd_and_hms(2026, 6, 26, 9, 30, 0).unwrap();
    let outcome = ingest_summary(
        &config,
        &tree,
        SummaryIngestInput {
            content: "Memory wiki git history now records summary-node seals.".to_string(),
            token_count: 64,
            entities: Vec::new(),
            topics: vec!["memory".to_string()],
            time_range_start: start,
            time_range_end: end,
            score: 0.8,
            child_labels: vec!["commit:abc123".to_string(), "issue:4142".to_string()],
            child_basenames: Vec::new(),
        },
    )
    .await
    .expect("ingest summary");

    let wiki_root = content_root.join("wiki");
    let repo = git2::Repository::open(&wiki_root).expect("wiki git repo should be initialized");
    let head = repo.head().expect("wiki head").peel_to_commit().unwrap();
    let tree_obj = head.tree().expect("wiki commit tree");

    let repo_summary_path = outcome
        .content_path
        .strip_prefix("wiki/")
        .expect("summary path should live under wiki/");
    tree_obj
        .get_path(std::path::Path::new(repo_summary_path))
        .expect("summary markdown should be tracked");
    assert!(
        tree_obj
            .get_path(std::path::Path::new("raw/not-tracked.md"))
            .is_err(),
        "git history should remain scoped to summaries"
    );

    let message = head.message().expect("commit message");
    assert!(message.contains("Seal memory tree github:tinyhumansai/openhuman L1 summaries"));
    assert!(message.contains("Reason: summary_ingest"));
    assert!(message.contains("Summary-Count: 1"));
    assert!(message.contains("Child-Count: 2"));
    assert!(message.contains("Token-Count: 64"));
    assert!(message.contains(&outcome.summary_id));
    assert!(message.contains(repo_summary_path));

    let head_id = head.id().to_string();
    let tagged = set_read_pointer_tag(&content_root, "agent:e2e", None)
        .expect("set timestamped read pointer tag");
    assert_eq!(tagged, head_id);
    assert_eq!(
        get_read_pointer_tag(&content_root, "agent:e2e")
            .expect("read latest pointer")
            .as_deref(),
        Some(head_id.as_str())
    );

    let tag_prefix = format!("refs/tags/read/{}/", hex::encode("agent:e2e".as_bytes()));
    let tags = repo.references().unwrap().fold(Vec::new(), |mut acc, r| {
        let r = r.unwrap();
        let name = r.name().unwrap();
        if name.starts_with(&tag_prefix) {
            acc.push(name.to_string());
        }
        acc
    });
    assert!(tags.iter().any(|name| name.ends_with("/latest")));
    assert!(tags.iter().any(|name| {
        let suffix = name.strip_prefix(&tag_prefix).unwrap_or_default();
        suffix.len() == "20260626T090000.000000000Z".len()
            && suffix.ends_with('Z')
            && suffix.contains('T')
    }));
}
