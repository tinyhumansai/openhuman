//! Focused raw integration coverage for memory-family modules.
//!
//! These tests avoid network and keep all state in per-test tempdirs. Run with
//! `--test-threads=1` because several memory surfaces use process-global
//! stores or cached SQLite connections.

use chrono::{TimeZone, Utc};
use serde_json::json;
use tempfile::TempDir;

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::memory::{
    ExtractionMode, IngestionState, MemoryIngestionConfig, MemoryIngestionRequest,
    NamespaceDocumentInput,
};
use openhuman_core::openhuman::memory_sources::status::{source_status, FreshnessLabel};
use openhuman_core::openhuman::memory_sources::{MemorySourceEntry, SourceKind};
use openhuman_core::openhuman::memory_store::chunks::store::upsert_chunks;
use openhuman_core::openhuman::memory_store::chunks::types::{
    approx_token_count, chunk_id, Chunk, Metadata, SourceKind as ChunkSourceKind, SourceRef,
};
use openhuman_core::openhuman::memory_sync::canonicalize::chat::{
    canonicalise as canonicalise_chat, ChatBatch, ChatMessage,
};
use openhuman_core::openhuman::memory_sync::canonicalize::document::{
    canonicalise as canonicalise_document, DocumentInput,
};
use openhuman_core::openhuman::memory_sync::canonicalize::email::{
    canonicalise as canonicalise_email, EmailMessage, EmailThread,
};
use openhuman_core::openhuman::memory_sync::composio::providers::{
    classify_unknown, find_curated, toolkit_from_slug, CuratedTool, ToolScope,
};
use openhuman_core::openhuman::memory_sync::{SyncOutcome, SyncPipelineKind};
use openhuman_core::openhuman::memory_tree::summarise::{
    fallback_summary, SummaryContext, SummaryInput,
};
use openhuman_core::openhuman::memory_tree::tree_runtime::store as tree_store;
use openhuman_core::openhuman::memory_tree::tree_runtime::{
    derive_node_ids, estimate_tokens, level_from_node_id, node_id_to_path, NodeLevel, TreeNode,
};
use openhuman_core::openhuman::threads::turn_state::{
    SubagentActivity, SubagentToolCall, ToolTimelineEntry, ToolTimelineStatus, TurnLifecycle,
    TurnPhase, TurnState, TurnStateStore,
};

fn config_in(tmp: &TempDir) -> Config {
    Config {
        workspace_dir: tmp.path().to_path_buf(),
        ..Config::default()
    }
}

fn source_entry(kind: SourceKind, id: &str) -> MemorySourceEntry {
    MemorySourceEntry {
        id: id.to_string(),
        kind,
        label: format!("{id} label"),
        enabled: true,
        toolkit: None,
        connection_id: None,
        path: None,
        glob: None,
        url: None,
        branch: None,
        paths: Vec::new(),
        query: None,
        since_days: None,
        max_items: None,
        max_commits: None,
        max_issues: None,
        max_prs: None,
        selector: None,
        max_tokens_per_sync: None,
        max_cost_per_sync_usd: None,
        sync_depth_days: None,
    }
}

fn tree_node(namespace: &str, node_id: &str, summary: &str) -> TreeNode {
    let ts = Utc.with_ymd_and_hms(2026, 5, 29, 12, 30, 0).unwrap();
    TreeNode {
        node_id: node_id.to_string(),
        namespace: namespace.to_string(),
        level: level_from_node_id(node_id),
        parent_id: openhuman_core::openhuman::memory_tree::tree_runtime::derive_parent_id(node_id),
        summary: summary.to_string(),
        token_count: estimate_tokens(summary),
        child_count: 0,
        created_at: ts,
        updated_at: ts,
        metadata: Some(json!({ "test": "memory_raw_coverage", "node": node_id }).to_string()),
    }
}

fn chunk(source_id: &str, seq: u32, timestamp_ms: i64, embedding_pending: bool) -> Chunk {
    let content = format!("memory raw coverage chunk {source_id} #{seq}");
    let ts = Utc.timestamp_millis_opt(timestamp_ms).unwrap();
    let mut metadata = Metadata::point_in_time(ChunkSourceKind::Document, source_id, "owner", ts);
    metadata.tags = vec!["coverage".into()];
    metadata.source_ref = Some(SourceRef::new(format!("file:///{source_id}/{seq}")));
    let mut chunk = Chunk {
        id: chunk_id(ChunkSourceKind::Document, source_id, seq, &content),
        content,
        metadata,
        token_count: approx_token_count(source_id),
        seq_in_source: seq,
        created_at: ts,
        partial_message: false,
    };
    if !embedding_pending {
        chunk.partial_message = true;
    }
    chunk
}

#[test]
fn memory_tree_store_round_trips_nodes_buffers_and_validation_edges() {
    let tmp = TempDir::new().expect("tempdir");
    let config = config_in(&tmp);
    let ns = "raw/coverage:tree";

    assert!(tree_store::validate_namespace("personal").is_ok());
    assert!(tree_store::validate_namespace(" ").is_err());
    assert!(tree_store::validate_namespace("../escape").is_err());
    assert!(tree_store::validate_namespace("/absolute").is_err());
    assert!(tree_store::validate_node_id("root").is_ok());
    assert!(tree_store::validate_node_id("2026/05/29/23").is_ok());
    assert!(tree_store::validate_node_id("2026/13").is_err());
    assert!(tree_store::validate_node_id("2026/05/32").is_err());
    assert!(tree_store::validate_node_id("2026/05/29/24").is_err());
    assert!(tree_store::validate_node_id("../root").is_err());

    for (node_id, summary) in [
        ("root", "Root summary for the workspace"),
        ("2026", "Year summary"),
        ("2026/05", "Month summary"),
        ("2026/05/29", "Day summary"),
        ("2026/05/29/12", "Hour leaf summary"),
    ] {
        tree_store::write_node(&config, &tree_node(ns, node_id, summary)).expect("write node");
    }

    let root = tree_store::read_node(&config, ns, "root")
        .expect("read root")
        .expect("root exists");
    assert_eq!(root.level, NodeLevel::Root);
    assert_eq!(root.parent_id, None);

    let root_children = tree_store::read_children(&config, ns, "root").expect("root children");
    assert_eq!(root_children.len(), 1);
    assert_eq!(root_children[0].node_id, "2026");
    let day_children = tree_store::read_children(&config, ns, "2026/05/29").expect("day children");
    assert_eq!(day_children[0].node_id, "2026/05/29/12");

    let ancestors = tree_store::read_ancestors(&config, ns, "2026/05/29/12").expect("ancestors");
    assert_eq!(
        ancestors
            .iter()
            .map(|n| n.node_id.as_str())
            .collect::<Vec<_>>(),
        vec!["2026/05/29", "2026/05", "2026", "root"]
    );

    let status = tree_store::get_tree_status(&config, ns).expect("status");
    assert_eq!(status.total_nodes, 5);
    assert_eq!(status.depth, 5);
    assert!(status.oldest_entry.is_some());
    assert!(status.newest_entry.is_some());

    let ts = Utc.with_ymd_and_hms(2026, 5, 29, 13, 0, 0).unwrap();
    let first =
        tree_store::buffer_write(&config, ns, "plain buffer", &ts, None).expect("buffer write");
    let second = tree_store::buffer_write(
        &config,
        ns,
        "frontmatter buffer",
        &ts,
        Some(&json!({ "source": "test" })),
    )
    .expect("buffer write with metadata");
    assert!(first.exists());
    assert!(second.exists());

    let buffered = tree_store::buffer_read(&config, ns).expect("buffer read");
    assert_eq!(buffered.len(), 2);
    assert!(buffered.iter().any(|(_, body)| body == "plain buffer"));
    assert!(buffered
        .iter()
        .any(|(_, body)| body == "frontmatter buffer"));

    let drained = tree_store::buffer_drain(&config, ns).expect("buffer drain");
    assert_eq!(drained.len(), 2);
    assert!(tree_store::buffer_read(&config, ns)
        .expect("buffer empty")
        .is_empty());

    let collected = tree_store::collect_root_summaries_with_caps(tmp.path(), 10, 12);
    assert_eq!(collected.len(), 1);
    assert!(collected[0].1.contains("Root summa"));
    assert!(collected[0].1.contains("truncated"));

    let deleted = tree_store::delete_tree(&config, ns).expect("delete tree");
    assert_eq!(deleted, 5);
    assert_eq!(
        tree_store::delete_tree(&config, ns).expect("delete missing"),
        0
    );
    assert!(tree_store::read_node(&config, ns, "root")
        .expect("read missing")
        .is_none());
}

#[test]
fn memory_tree_types_and_fallback_summary_cover_budget_and_legacy_parse_paths() {
    let ts = Utc.with_ymd_and_hms(2026, 5, 29, 9, 8, 7).unwrap();
    let (hour, day, month, year, root) = derive_node_ids(&ts);
    assert_eq!(root, "root");
    assert_eq!(year, "2026");
    assert_eq!(month, "2026/05");
    assert_eq!(day, "2026/05/29");
    assert_eq!(hour, "2026/05/29/09");
    assert_eq!(node_id_to_path("root").to_string_lossy(), "root.md");
    assert!(node_id_to_path("2026/05/29/09")
        .to_string_lossy()
        .ends_with("2026/05/29/09.md"));
    assert_eq!(NodeLevel::Hour.parent_level(), Some(NodeLevel::Day));
    assert!(NodeLevel::Hour.is_leaf());
    assert_eq!(NodeLevel::Root.max_tokens(), 20_000);
    assert_eq!(NodeLevel::from_str_label("month"), Some(NodeLevel::Month));
    assert_eq!(NodeLevel::from_str_label("bogus"), None);

    let legacy = "---\nlevel: hour\nparent_id: \"2026/05/29\"\ntoken_count: 3\n---\n\nlegacy body";
    let parsed = tree_store::parse_node_markdown_pub(legacy, "legacy", "2026/05/29/09")
        .expect("legacy parse");
    assert_eq!(parsed.created_at.timestamp(), 0);
    assert_eq!(parsed.updated_at, parsed.created_at);
    assert_eq!(parsed.summary, "legacy body");

    let inputs = vec![
        SummaryInput {
            id: "blank".into(),
            content: "   ".into(),
            token_count: 0,
            entities: vec!["ignored".into()],
            topics: vec![],
            time_range_start: ts,
            time_range_end: ts,
            score: 0.1,
        },
        SummaryInput {
            id: "long".into(),
            content: "alpha beta gamma delta epsilon zeta eta theta".repeat(20),
            token_count: 200,
            entities: vec![],
            topics: vec!["planning".into()],
            time_range_start: ts,
            time_range_end: ts,
            score: 0.9,
        },
    ];
    let out = fallback_summary(&inputs, 8);
    assert!(out.content.starts_with("— alpha"));
    assert!(out.token_count <= 9);
    assert!(out.entities.is_empty());
    assert!(out.topics.is_empty());

    let ctx = SummaryContext {
        tree_id: "tree-coverage",
        tree_kind: openhuman_core::openhuman::memory_store::trees::types::TreeKind::Global,
        target_level: 2,
        token_budget: 128,
    };
    assert_eq!(ctx.tree_id, "tree-coverage");
    assert_eq!(ctx.target_level, 2);
}

#[tokio::test]
async fn memory_sources_status_counts_folder_and_composio_prefixes() {
    let tmp = TempDir::new().expect("tempdir");
    let config = config_in(&tmp);
    let folder_source_id = "mem_src:folder-alpha:file-a.md";
    let gmail_source_id = "gmail:conn-1:message-1";

    let now = Utc::now().timestamp_millis();
    upsert_chunks(
        &config,
        &[
            chunk(folder_source_id, 0, now - 1_000, true),
            chunk(folder_source_id, 1, now - 400_000, false),
            chunk(gmail_source_id, 0, now - 2_000, true),
        ],
    )
    .expect("upsert chunks");

    let mut folder = source_entry(SourceKind::Folder, "folder-alpha");
    folder.path = Some(tmp.path().to_string_lossy().into_owned());
    let folder_status = source_status(&config, &folder)
        .await
        .expect("folder status");
    assert_eq!(folder_status.source_id, "folder-alpha");
    assert_eq!(folder_status.chunks_synced, 2);
    assert_eq!(folder_status.chunks_pending, 2);
    assert_eq!(folder_status.freshness, FreshnessLabel::Active);

    let mut composio = source_entry(SourceKind::Composio, "gmail-source");
    composio.toolkit = Some("gmail".into());
    composio.connection_id = Some("conn-1".into());
    let composio_status = source_status(&config, &composio)
        .await
        .expect("composio status");
    assert_eq!(composio_status.chunks_synced, 1);
    assert_eq!(composio_status.freshness, FreshnessLabel::Active);

    let mut missing_toolkit = composio.clone();
    missing_toolkit.id = "missing-toolkit".into();
    missing_toolkit.toolkit = None;
    let missing = source_status(&config, &missing_toolkit)
        .await
        .expect("missing toolkit status");
    assert_eq!(missing.chunks_synced, 0);
    assert_eq!(missing.freshness, FreshnessLabel::Idle);

    assert_eq!(FreshnessLabel::from_age_ms(None, now), FreshnessLabel::Idle);
    assert_eq!(
        FreshnessLabel::from_age_ms(Some(now - 60_000), now),
        FreshnessLabel::Recent
    );
    assert_eq!(
        FreshnessLabel::from_age_ms(Some(now - 600_000), now),
        FreshnessLabel::Idle
    );
}

#[test]
fn memory_sources_validation_and_sync_classification_edges() {
    let mut entry = source_entry(SourceKind::Folder, "src-folder");
    assert_eq!(entry.kind.as_str(), "folder");
    assert!(entry.validate().is_err());
    entry.path = Some("/tmp/notes".into());
    assert!(entry.validate().is_ok());

    let mut github = source_entry(SourceKind::GithubRepo, "src-github");
    assert!(github.validate().is_err());
    github.url = Some("https://github.com/tinyhumansai/openhuman".into());
    assert!(github.validate().is_ok());

    let mut twitter = source_entry(SourceKind::TwitterQuery, "src-twitter");
    assert!(twitter.validate().is_err());
    twitter.query = Some("openhuman".into());
    assert!(twitter.validate().is_ok());

    let mut rss = source_entry(SourceKind::RssFeed, "src-rss");
    rss.url = Some("https://example.com/feed.xml".into());
    assert_eq!(rss.kind.as_str(), "rss_feed");
    assert!(rss.validate().is_ok());

    let mut web = source_entry(SourceKind::WebPage, "src-web");
    web.url = Some("https://example.com/page".into());
    assert_eq!(web.kind.as_str(), "web_page");
    assert!(web.validate().is_ok());

    let mut composio = source_entry(SourceKind::Composio, "src-composio");
    composio.toolkit = Some("gmail".into());
    assert!(composio.validate().is_err());
    composio.connection_id = Some("conn".into());
    assert!(composio.validate().is_ok());

    assert_eq!(classify_unknown("GMAIL_DELETE_DRAFT"), ToolScope::Admin);
    assert_eq!(classify_unknown("NOTION_CREATE_PAGE"), ToolScope::Write);
    assert_eq!(classify_unknown("GMAIL_FETCH_EMAILS"), ToolScope::Read);
    assert_eq!(
        toolkit_from_slug(" MICROSOFT_TEAMS_SEND "),
        Some("microsoft_teams".into())
    );
    assert_eq!(toolkit_from_slug(""), None);
    let catalog = [CuratedTool {
        slug: "GMAIL_SEND_EMAIL",
        scope: ToolScope::Write,
    }];
    assert_eq!(
        find_curated(&catalog, "gmail_send_email").unwrap().scope,
        ToolScope::Write
    );
    assert!(find_curated(&catalog, "GMAIL_DELETE_EMAIL").is_none());

    assert_eq!(ToolScope::Admin.as_str(), "admin");
    assert_eq!(SyncPipelineKind::Composio.as_str(), "composio");
    assert_eq!(SyncPipelineKind::Workspace.as_str(), "workspace");
    assert_eq!(SyncPipelineKind::Mcp.as_str(), "mcp");
    let outcome = SyncOutcome {
        records_ingested: 3,
        more_pending: true,
        note: Some("paged".into()),
    };
    let encoded = serde_json::to_value(&outcome).expect("sync outcome json");
    assert_eq!(encoded["records_ingested"], 3);
    assert_eq!(encoded["more_pending"], true);
}

#[test]
fn memory_sync_canonicalizers_sort_clean_and_preserve_provenance() {
    let t1 = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
    let t2 = Utc.timestamp_millis_opt(1_700_000_010_000).unwrap();

    assert!(canonicalise_chat(
        "slack:empty",
        "alice",
        &[],
        ChatBatch {
            platform: "slack".into(),
            channel_label: "#empty".into(),
            messages: vec![],
        },
    )
    .expect("empty chat")
    .is_none());

    let chat = canonicalise_chat(
        "slack:#eng",
        "alice@example.com",
        &["eng".into()],
        ChatBatch {
            platform: "slack".into(),
            channel_label: "#eng".into(),
            messages: vec![
                ChatMessage {
                    author: "Bob".into(),
                    timestamp: t2,
                    text: "second".into(),
                    source_ref: Some("slack://second".into()),
                },
                ChatMessage {
                    author: "Alice".into(),
                    timestamp: t1,
                    text: " first ".into(),
                    source_ref: Some("slack://first".into()),
                },
            ],
        },
    )
    .expect("chat")
    .expect("chat output");
    assert!(chat.markdown.find("first").unwrap() < chat.markdown.find("second").unwrap());
    assert_eq!(chat.metadata.time_range, (t1, t2));
    assert_eq!(chat.metadata.source_ref.unwrap().value, "slack://first");

    let email = canonicalise_email(
        "gmail:thread",
        "alice@example.com",
        &["inbox".into()],
        EmailThread {
            provider: "gmail".into(),
            thread_subject: "Launch".into(),
            messages: vec![
                EmailMessage {
                    from: "bob@example.com".into(),
                    to: vec!["alice@example.com".into()],
                    cc: vec!["carol@example.com".into()],
                    subject: "Launch".into(),
                    sent_at: t2,
                    body: "Reply body\n\nUnsubscribe https://example.com".into(),
                    source_ref: Some("<second@example.com>".into()),
                    list_unsubscribe: Some("<mailto:unsubscribe@example.com>".into()),
                },
                EmailMessage {
                    from: "alice@example.com".into(),
                    to: vec!["bob@example.com".into()],
                    cc: vec![],
                    subject: "Re: Launch".into(),
                    sent_at: t1,
                    body: "Original body".into(),
                    source_ref: Some(" ".into()),
                    list_unsubscribe: None,
                },
            ],
        },
    )
    .expect("email")
    .expect("email output");
    assert!(
        email.markdown.find("Original body").unwrap() < email.markdown.find("Reply body").unwrap()
    );
    assert!(email.markdown.contains("Cc: carol@example.com"));
    assert!(email
        .markdown
        .contains("List-Unsubscribe: <mailto:unsubscribe@example.com>"));
    assert!(!email.markdown.contains("https://example.com"));
    assert!(email.metadata.source_ref.is_none());

    assert!(canonicalise_document(
        "doc-empty",
        "alice",
        &[],
        DocumentInput {
            provider: "notion".into(),
            title: " ".into(),
            body: " ".into(),
            modified_at: t1,
            source_ref: None,
        },
        None,
    )
    .expect("empty doc")
    .is_none());

    let doc_json = json!({
        "title": "Plan",
        "body": "Plan body",
        "modified_at": "1700000000000",
        "source_ref": "notion://page/1"
    });
    let doc_input: DocumentInput = serde_json::from_value(doc_json).expect("document input");
    assert_eq!(doc_input.provider, "unknown");
    let doc = canonicalise_document("doc-1", "alice", &["plans".into()], doc_input, None)
        .expect("document")
        .expect("document output");
    assert_eq!(doc.metadata.timestamp.timestamp_millis(), 1_700_000_000_000);
    assert_eq!(doc.metadata.source_ref.unwrap().value, "notion://page/1");
    assert_eq!(doc.markdown, "Plan body\n");
}

#[tokio::test]
async fn memory_ingestion_state_and_request_models_report_edges() {
    let state = IngestionState::new();
    state.enqueue();
    state.enqueue();
    {
        let _guard = state.acquire().await;
        state.dequeue();
        state.mark_running("doc-1", "Coverage Doc", "coverage-ns");
        let running = state.snapshot();
        assert!(running.running);
        assert_eq!(running.queue_depth, 1);
        assert_eq!(running.current_title.as_deref(), Some("Coverage Doc"));
    }
    state.mark_completed("doc-1", false, 1_700_000_000_000);
    let completed = state.snapshot();
    assert!(!completed.running);
    assert_eq!(completed.last_document_id.as_deref(), Some("doc-1"));
    assert_eq!(completed.last_success, Some(false));

    let cfg = MemoryIngestionConfig {
        model_name: "local-model".into(),
        extraction_mode: ExtractionMode::Chunk,
        entity_threshold: 0.42,
        relation_threshold: 0.37,
        adjacency_threshold: 0.51,
        batch_size: 7,
    };
    let req = MemoryIngestionRequest {
        document: NamespaceDocumentInput {
            namespace: "coverage".into(),
            key: "doc-key".into(),
            title: "Coverage Doc".into(),
            content: "Alice collaborates with Bob on OpenHuman memory tests.".into(),
            source_type: "test".into(),
            priority: "medium".into(),
            tags: vec!["coverage".into()],
            metadata: json!({ "kind": "test" }),
            category: "core".into(),
            session_id: Some("session-1".into()),
            document_id: Some("doc-1".into()),
            taint: openhuman_core::openhuman::memory::MemoryTaint::Internal,
        },
        config: cfg.clone(),
    };
    assert_eq!(req.document.document_id.as_deref(), Some("doc-1"));
    assert_eq!(req.config.batch_size, 7);
    assert_eq!(req.config.extraction_mode, cfg.extraction_mode);
}

#[test]
fn threads_turn_state_store_skips_corrupt_entries_and_marks_interrupted() {
    let tmp = TempDir::new().expect("tempdir");
    let store = TurnStateStore::new(tmp.path().to_path_buf());
    assert!(store.list().expect("initial list").is_empty());
    assert!(!store.delete("missing").expect("delete missing"));
    assert_eq!(store.clear_all().expect("clear missing"), 0);

    let mut first = TurnState::started("thread-a", "req-a", 4, "2026-05-29T12:00:00Z");
    first.lifecycle = TurnLifecycle::Streaming;
    first.iteration = 2;
    first.phase = Some(TurnPhase::ToolUse);
    first.active_tool = Some("memory.search".into());
    first.tool_timeline.push(ToolTimelineEntry {
        id: "tool-1".into(),
        name: "memory.search".into(),
        round: 1,
        status: ToolTimelineStatus::Running,
        args_buffer: Some("{\"query\":\"coverage\"}".into()),
        display_name: Some("Search Memory".into()),
        detail: None,
        source_tool_name: Some("memory.search".into()),
        subagent: Some(SubagentActivity {
            task_id: "task-1".into(),
            agent_id: "researcher".into(),
            status: Some("running".into()),
            mode: Some("read".into()),
            dedicated_thread: Some(false),
            child_iteration: Some(1),
            child_max_iterations: Some(3),
            iterations: Some(1),
            elapsed_ms: Some(25),
            output_chars: Some(128),
            worker_thread_id: None,
            tool_calls: vec![SubagentToolCall {
                call_id: "call-1".into(),
                tool_name: "memory.search".into(),
                status: ToolTimelineStatus::Success,
                iteration: Some(1),
                elapsed_ms: Some(20),
                output_chars: Some(64),
                display_name: None,
                detail: None,
            }],
            transcript: vec![],
        }),
    });
    let second = TurnState::started("thread-b", "req-b", 2, "2026-05-29T12:01:00Z");
    store.put(&first).expect("put first");
    store.put(&second).expect("put second");

    let loaded = store
        .get("thread-a")
        .expect("get first")
        .expect("first exists");
    assert_eq!(loaded.active_tool.as_deref(), Some("memory.search"));
    assert_eq!(loaded.tool_timeline[0].status, ToolTimelineStatus::Running);

    let dir = tmp
        .path()
        .join("memory")
        .join("conversations")
        .join("turn_states");
    std::fs::write(dir.join("corrupt.json"), "{not-json").expect("write corrupt snapshot");
    let listed = store.list().expect("list skips corrupt");
    assert_eq!(listed.len(), 2);

    let interrupted = store
        .mark_all_interrupted("2026-05-29T12:02:00Z")
        .expect("mark interrupted");
    assert_eq!(interrupted, 2);
    let after = store.get("thread-a").expect("get after").expect("exists");
    assert_eq!(after.lifecycle, TurnLifecycle::Interrupted);
    assert!(after.active_tool.is_none());
    assert_eq!(after.updated_at, "2026-05-29T12:02:00Z");
    assert_eq!(
        store
            .mark_all_interrupted("2026-05-29T12:03:00Z")
            .expect("idempotent mark"),
        0
    );

    assert!(store.delete("thread-b").expect("delete thread-b"));
    assert!(store
        .get("thread-b")
        .expect("missing after delete")
        .is_none());
    assert_eq!(store.clear_all().expect("clear all"), 2);
    assert!(store.list().expect("empty after clear").is_empty());
}
