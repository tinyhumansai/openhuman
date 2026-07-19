use super::*;
use crate::openhuman::composio::providers::sync_state::KV_NAMESPACE;
use crate::openhuman::embeddings::NoopEmbedding;
use crate::openhuman::memory::ingest_pipeline::ingest_chat;
use crate::openhuman::memory_queue::drain_until_idle;
use crate::openhuman::memory_store::content::raw::{write_raw_items, RawItem, RawKind};
use crate::openhuman::memory_store::unified::UnifiedMemory;
use crate::openhuman::memory_sync::canonicalize::chat::{ChatBatch, ChatMessage};
use chrono::{TimeZone, Utc};
use rusqlite::params;
use std::sync::Arc;
use tempfile::TempDir;

fn test_config() -> (TempDir, Config) {
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    // Point config_path inside the tempdir so any persistence during
    // tests stays inside disposable workspace state.
    cfg.config_path = tmp.path().join("config.toml");
    cfg.memory_tree.embedding_endpoint = None;
    cfg.memory_tree.embedding_model = None;
    cfg.memory_tree.embedding_strict = false;
    // Default llm is Cloud — but the cloud provider needs a bearer
    // token to actually fire. Tests that exercise the LLM path
    // override either the backend or the extractor. The read RPCs
    // below don't touch the LLM, so this default is fine.
    (tmp, cfg)
}

async fn seed_chat_chunk(cfg: &Config, source: &str, body: &str) {
    let batch = ChatBatch {
        platform: "slack".into(),
        channel_label: source.into(),
        messages: vec![ChatMessage {
            author: "alice".into(),
            timestamp: Utc.timestamp_millis_opt(1_700_000_000_000).unwrap(),
            text: body.into(),
            source_ref: Some("slack://x".into()),
        }],
    };
    ingest_chat(cfg, source, "alice", vec![], batch)
        .await
        .unwrap();
}

async fn seed_slack_chunk_with_raw_archive(cfg: &Config) -> String {
    let timestamp = Utc.timestamp_opt(1_700_000_000, 0).single().unwrap();
    write_raw_items(
        &cfg.memory_tree_content_root(),
        "slack:conn-slack-1",
        &[RawItem {
            uid: "1700000000.000100",
            created_at_ms: timestamp.timestamp_millis(),
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
            timestamp,
            text: "Phoenix migration launch window is Friday at 22:00 UTC.".into(),
            source_ref: Some("slack://archives/C123/1700000000.000100".into()),
        }],
    };
    ingest_chat(
        cfg,
        "slack:conn-slack-1",
        "alice",
        vec!["slack".into(), "ingested".into()],
        batch,
    )
    .await
    .expect("seed slack ingest");
    drain_until_idle(cfg).await.expect("drain slack ingest");

    list_chunks_rpc(cfg, ChunkFilter::default())
        .await
        .expect("list chunks")
        .value
        .chunks
        .into_iter()
        .find(|chunk| chunk.source_id == "slack:conn-slack-1")
        .expect("seeded slack chunk")
        .id
}

fn update_chunk_timestamp(cfg: &Config, chunk_id: &str, timestamp_ms: i64) {
    with_connection(cfg, |conn| {
        conn.execute(
            "UPDATE mem_tree_chunks
                SET timestamp_ms = ?1,
                    time_range_start_ms = ?1,
                    time_range_end_ms = ?1
              WHERE id = ?2",
            params![timestamp_ms, chunk_id],
        )?;
        Ok(())
    })
    .unwrap();
}

fn insert_raw_chunk(
    cfg: &Config,
    id: &str,
    source_kind: &str,
    source_id: &str,
    timestamp_ms: i64,
    tags_json: &str,
    content: &str,
    token_count: i64,
) {
    with_connection(cfg, |conn| {
        conn.execute(
            "INSERT INTO mem_tree_chunks (
                id, source_kind, source_id, source_ref, owner, timestamp_ms,
                time_range_start_ms, time_range_end_ms, tags_json, content,
                token_count, seq_in_source, created_at_ms, lifecycle_status, content_path
             ) VALUES (?1, ?2, ?3, NULL, 'tester', ?4, ?4, ?4, ?5, ?6, ?7, 0, ?4, 'seeded', NULL)",
            params![
                id,
                source_kind,
                source_id,
                timestamp_ms,
                tags_json,
                content,
                token_count
            ],
        )?;
        Ok(())
    })
    .unwrap();
}

#[tokio::test]
async fn list_chunks_returns_seeded_chunk() {
    let (_tmp, cfg) = test_config();
    seed_chat_chunk(&cfg, "slack:#eng", "hello @alice phoenix migration").await;
    let resp = list_chunks_rpc(&cfg, ChunkFilter::default())
        .await
        .unwrap()
        .value;
    assert!(!resp.chunks.is_empty());
    assert_eq!(resp.total, resp.chunks.len() as u64);
}

#[tokio::test]
async fn list_chunks_filters_by_source_id() {
    let (_tmp, cfg) = test_config();
    seed_chat_chunk(&cfg, "slack:#a", "alpha").await;
    seed_chat_chunk(&cfg, "slack:#b", "beta").await;
    let only_a = list_chunks_rpc(
        &cfg,
        ChunkFilter {
            source_ids: Some(vec!["slack:#a".into()]),
            ..ChunkFilter::default()
        },
    )
    .await
    .unwrap()
    .value;
    assert!(only_a.chunks.iter().all(|c| c.source_id == "slack:#a"));
    assert!(only_a.total >= 1);
}

#[tokio::test]
async fn list_chunks_query_substring_works() {
    let (_tmp, cfg) = test_config();
    seed_chat_chunk(&cfg, "slack:#eng", "phoenix migration ships friday").await;
    seed_chat_chunk(&cfg, "slack:#eng", "different unrelated text").await;
    let resp = list_chunks_rpc(
        &cfg,
        ChunkFilter {
            query: Some("phoenix".into()),
            ..ChunkFilter::default()
        },
    )
    .await
    .unwrap()
    .value;
    assert!(resp.chunks.iter().any(|c| {
        c.content_preview
            .as_deref()
            .unwrap_or("")
            .contains("phoenix")
    }));
}

#[tokio::test]
async fn list_chunks_filters_by_source_kind_and_applies_limit_offset() {
    let (_tmp, cfg) = test_config();
    seed_chat_chunk(&cfg, "slack:#a", "first chat").await;
    seed_chat_chunk(&cfg, "slack:#b", "second chat").await;

    let filtered = list_chunks_rpc(
        &cfg,
        ChunkFilter {
            source_kinds: Some(vec!["chat".into()]),
            limit: Some(1),
            offset: Some(1),
            ..ChunkFilter::default()
        },
    )
    .await
    .unwrap()
    .value;
    assert_eq!(filtered.chunks.len(), 1);
    assert_eq!(filtered.total, 2);
    assert!(filtered.chunks.iter().all(|c| c.source_kind == "chat"));
}

#[tokio::test]
async fn list_chunks_filters_by_entity_id_and_time_window() {
    let (_tmp, cfg) = test_config();
    seed_chat_chunk(&cfg, "slack:#eng", "alice@example.com handles phoenix").await;
    seed_chat_chunk(&cfg, "slack:#eng", "bob@example.com handles atlas").await;

    let seeded = list_chunks_rpc(&cfg, ChunkFilter::default())
        .await
        .unwrap()
        .value
        .chunks;
    let alice = seeded
        .iter()
        .find(|chunk| {
            chunk
                .content_preview
                .as_deref()
                .unwrap_or("")
                .contains("alice@example.com")
        })
        .expect("alice chunk present");
    let bob = seeded
        .iter()
        .find(|chunk| {
            chunk
                .content_preview
                .as_deref()
                .unwrap_or("")
                .contains("bob@example.com")
        })
        .expect("bob chunk present");

    update_chunk_timestamp(&cfg, &alice.id, 1_700_000_000_100);
    update_chunk_timestamp(&cfg, &bob.id, 1_700_000_000_900);

    let filtered = list_chunks_rpc(
        &cfg,
        ChunkFilter {
            entity_ids: Some(vec!["email:alice@example.com".into()]),
            since_ms: Some(1_700_000_000_000),
            until_ms: Some(1_700_000_000_500),
            ..ChunkFilter::default()
        },
    )
    .await
    .unwrap()
    .value;

    assert_eq!(filtered.total, 1);
    assert_eq!(filtered.chunks.len(), 1);
    assert_eq!(filtered.chunks[0].id, alice.id);
}

#[tokio::test]
async fn list_chunks_ignores_empty_filter_lists_and_blank_query() {
    let (_tmp, cfg) = test_config();
    seed_chat_chunk(&cfg, "slack:#a", "alpha").await;
    seed_chat_chunk(&cfg, "slack:#b", "beta").await;

    let resp = list_chunks_rpc(
        &cfg,
        ChunkFilter {
            source_kinds: Some(vec![]),
            source_ids: Some(vec![]),
            entity_ids: Some(vec![]),
            query: Some("   ".into()),
            limit: Some(10),
            ..ChunkFilter::default()
        },
    )
    .await
    .unwrap()
    .value;

    assert_eq!(resp.total, 2);
    assert_eq!(resp.chunks.len(), 2);
}

#[tokio::test]
async fn list_chunks_normalizes_invalid_tags_negative_tokens_and_empty_content() {
    let (_tmp, cfg) = test_config();
    insert_raw_chunk(
        &cfg,
        "raw-empty",
        "document",
        "notion:page-1",
        1_700_000_000_123,
        "not-json",
        "",
        -7,
    );

    let resp = list_chunks_rpc(&cfg, ChunkFilter::default())
        .await
        .unwrap()
        .value;
    let row = resp
        .chunks
        .into_iter()
        .find(|chunk| chunk.id == "raw-empty")
        .expect("raw chunk listed");

    assert_eq!(row.token_count, 0);
    assert_eq!(row.tags, Vec::<String>::new());
    assert_eq!(row.content_preview, None);
    assert!(!row.has_embedding);
}

#[tokio::test]
async fn list_sources_aggregates() {
    let (_tmp, cfg) = test_config();
    seed_chat_chunk(&cfg, "slack:#a", "x").await;
    seed_chat_chunk(&cfg, "slack:#a", "y").await;
    seed_chat_chunk(&cfg, "slack:#b", "z").await;
    let sources = list_sources_rpc(&cfg, None).await.unwrap().value;
    let a = sources
        .iter()
        .find(|s| s.source_id == "slack:#a")
        .expect("expected slack:#a");
    let b = sources
        .iter()
        .find(|s| s.source_id == "slack:#b")
        .expect("expected slack:#b");
    assert_eq!(a.chunk_count, 2);
    assert_eq!(b.chunk_count, 1);
}

#[tokio::test]
async fn list_sources_formats_email_threads_with_trimmed_user_hint() {
    let (_tmp, cfg) = test_config();
    insert_raw_chunk(
        &cfg,
        "email-thread",
        "email",
        "gmail:Alice@Example.com|bob@example.com|carol@example.com",
        1_700_000_000_123,
        "[]",
        "thread body",
        12,
    );

    let sources = list_sources_rpc(&cfg, Some(" alice@example.com ".into()))
        .await
        .unwrap()
        .value;
    let source = sources
        .iter()
        .find(|row| row.source_id == "gmail:Alice@Example.com|bob@example.com|carol@example.com")
        .expect("email thread source present");
    assert_eq!(source.display_name, "bob@example.com, carol@example.com");
}

#[tokio::test]
async fn entity_index_for_returns_extracted_entities() {
    let (_tmp, cfg) = test_config();
    seed_chat_chunk(&cfg, "slack:#eng", "alice@example.com owns it").await;
    // Find the chunk we just seeded.
    let chunks = list_chunks_rpc(&cfg, ChunkFilter::default())
        .await
        .unwrap()
        .value
        .chunks;
    let id = &chunks[0].id;
    let refs = entity_index_for_rpc(&cfg, id.clone()).await.unwrap().value;
    assert!(
        refs.iter().any(|r| r.entity_id.contains("alice")),
        "expected alice entity in index, got: {refs:?}"
    );
}

#[tokio::test]
async fn chunks_for_entity_returns_leaf_chunk_ids_only() {
    let (_tmp, cfg) = test_config();
    seed_chat_chunk(&cfg, "slack:#eng", "alice@example.com owns it").await;
    let chunk_id = list_chunks_rpc(&cfg, ChunkFilter::default())
        .await
        .unwrap()
        .value
        .chunks[0]
        .id
        .clone();

    let rows = chunks_for_entity_rpc(&cfg, "email:alice@example.com".into())
        .await
        .unwrap()
        .value;
    assert_eq!(rows, vec![chunk_id]);
}

#[tokio::test]
async fn top_entities_returns_most_frequent() {
    let (_tmp, cfg) = test_config();
    seed_chat_chunk(&cfg, "slack:#a", "alice@example.com x").await;
    seed_chat_chunk(&cfg, "slack:#b", "alice@example.com y").await;
    seed_chat_chunk(&cfg, "slack:#c", "bob@example.com z").await;
    let top = top_entities_rpc(&cfg, Some("email".into()), 10)
        .await
        .unwrap()
        .value;
    assert!(top
        .iter()
        .any(|e| e.entity_id == "email:alice@example.com" && e.count >= 2));
}

#[tokio::test]
async fn delete_chunk_removes_chunk_and_dependent_rows() {
    let (_tmp, cfg) = test_config();
    seed_chat_chunk(&cfg, "slack:#eng", "alice@example.com owns it").await;
    let chunks = list_chunks_rpc(&cfg, ChunkFilter::default())
        .await
        .unwrap()
        .value
        .chunks;
    let id = chunks[0].id.clone();
    let resp = delete_chunk_rpc(&cfg, id.clone()).await.unwrap().value;
    assert!(resp.deleted);
    // Re-list — the chunk should be gone.
    let after = list_chunks_rpc(&cfg, ChunkFilter::default())
        .await
        .unwrap()
        .value;
    assert!(after.chunks.iter().all(|c| c.id != id));
}

#[tokio::test]
async fn delete_missing_chunk_is_idempotent() {
    let (_tmp, cfg) = test_config();
    let resp = delete_chunk_rpc(&cfg, "does-not-exist".into())
        .await
        .unwrap()
        .value;
    assert!(!resp.deleted);
    assert_eq!(resp.score_rows_removed, 0);
}

#[tokio::test]
async fn chunk_score_returns_breakdown_after_ingest() {
    let (_tmp, cfg) = test_config();
    seed_chat_chunk(
        &cfg,
        "slack:#eng",
        "alice@example.com owns the phoenix migration",
    )
    .await;
    let chunks = list_chunks_rpc(&cfg, ChunkFilter::default())
        .await
        .unwrap()
        .value
        .chunks;
    let id = &chunks[0].id;
    let breakdown = chunk_score_rpc(&cfg, id.clone()).await.unwrap().value;
    assert!(breakdown.is_some(), "expected score row after ingest");
    let b = breakdown.unwrap();
    assert!(b.signals.iter().any(|s| s.name == "metadata_weight"));
    assert!(b.threshold > 0.0);
}

#[tokio::test]
async fn search_returns_matching_chunks() {
    let (_tmp, cfg) = test_config();
    seed_chat_chunk(&cfg, "slack:#eng", "phoenix migration scheduled friday").await;
    seed_chat_chunk(&cfg, "slack:#eng", "different unrelated text").await;
    let hits = search_rpc(&cfg, "phoenix".into(), 10).await.unwrap().value;
    assert!(hits.iter().any(|c| {
        c.content_preview
            .as_deref()
            .unwrap_or("")
            .contains("phoenix")
    }));
}

#[tokio::test]
async fn read_chunk_row_returns_preview_and_metadata() {
    let (_tmp, cfg) = test_config();
    seed_chat_chunk(
        &cfg,
        "slack:#eng",
        "phoenix migration scheduled friday with context and source refs",
    )
    .await;
    let chunk = list_chunks_rpc(&cfg, ChunkFilter::default())
        .await
        .unwrap()
        .value
        .chunks
        .into_iter()
        .next()
        .expect("seeded chunk");

    let row = read_chunk_row(&cfg, &chunk.id).unwrap().expect("chunk row");
    assert_eq!(row.id, chunk.id);
    assert_eq!(row.source_kind, "chat");
    assert_eq!(row.source_id, "slack:#eng");
    assert_eq!(row.source_ref.as_deref(), Some("slack://x"));
    assert_eq!(row.owner, "alice");
    assert_eq!(row.lifecycle_status, "pending_extraction");
    assert!(row.content_path.is_some());
    assert!(row
        .content_preview
        .as_deref()
        .unwrap_or("")
        .contains("phoenix migration scheduled friday"));
}

#[tokio::test]
async fn read_chunk_row_falls_back_to_sqlite_preview_when_file_missing() {
    let (_tmp, cfg) = test_config();
    let body = "sqlite preview survives missing file";
    seed_chat_chunk(&cfg, "slack:#eng", body).await;
    let chunk = list_chunks_rpc(&cfg, ChunkFilter::default())
        .await
        .unwrap()
        .value
        .chunks
        .into_iter()
        .next()
        .expect("seeded chunk");

    let rel_path = chunk.content_path.clone().expect("content path present");
    let abs_path = cfg.memory_tree_content_root().join(rel_path);
    std::fs::remove_file(&abs_path).expect("remove chunk file");

    let row = read_chunk_row(&cfg, &chunk.id).unwrap().expect("chunk row");
    assert_eq!(row.content_path, chunk.content_path);
    assert!(row.content_preview.as_deref().unwrap_or("").contains(body));
}

#[tokio::test]
async fn flush_now_enqueues_once_and_reports_stale_buffers() {
    let (_tmp, cfg) = test_config();
    seed_chat_chunk(
        &cfg,
        "slack:#eng",
        "Phoenix migration ships Friday after the release checklist closes.",
    )
    .await;
    drain_until_idle(&cfg).await.expect("drain jobs");

    let first = flush_now_rpc(&cfg).await.expect("flush_now first");
    assert!(first.value.enqueued, "first flush should enqueue work");
    assert!(
        first.value.stale_buffers >= 1,
        "expected at least one stale buffer after ingest"
    );

    let second = flush_now_rpc(&cfg).await.expect("flush_now second");
    assert!(
        !second.value.enqueued,
        "same 3-hour window should dedupe duplicate flush triggers"
    );
    assert!(
        second.value.stale_buffers >= 1,
        "deduped flush should still report current stale buffer count"
    );
}

#[tokio::test]
async fn reset_tree_preserves_raw_archive_and_source_registry() {
    let (_tmp, cfg) = test_config();
    let chunk_id = seed_slack_chunk_with_raw_archive(&cfg).await;
    let content_root = cfg.memory_tree_content_root();
    let raw_file = content_root
        .join("raw")
        .join("slack-conn-slack-1")
        .join("chats")
        .join("1700000000000_1700000000.000100.md");
    let source_file = content_root
        .join("raw")
        .join("slack-conn-slack-1")
        .join("_source.md");
    assert!(raw_file.exists(), "raw archive should exist before reset");
    assert!(
        source_file.exists(),
        "source registry should exist before reset"
    );

    let stale_summary = content_root
        .join("wiki")
        .join("summaries")
        .join("source-slack-conn-slack-1")
        .join("L1")
        .join("summary-stale.md");
    std::fs::create_dir_all(
        stale_summary
            .parent()
            .expect("stale summary parent should exist"),
    )
    .expect("create stale summary dir");
    std::fs::write(&stale_summary, "stale summary body").expect("write stale summary");
    assert!(stale_summary.exists(), "stale summary fixture should exist");

    let outcome = reset_tree_rpc(&cfg).await.expect("reset_tree");
    assert_eq!(outcome.value.chunks_requeued, 1);
    assert_eq!(outcome.value.jobs_enqueued, 1);
    assert!(
        outcome.value.tree_rows_deleted >= 1,
        "buffer/tree rows should be removed during reset"
    );

    let row = read_chunk_row(&cfg, &chunk_id)
        .expect("read chunk row")
        .expect("chunk row present after reset");
    assert_eq!(row.lifecycle_status, "pending_extraction");
    assert!(raw_file.exists(), "raw archive must survive reset_tree");
    assert!(
        source_file.exists(),
        "source registry must survive reset_tree"
    );
    assert!(
        !content_root.join("wiki").join("summaries").exists(),
        "derived wiki summaries should be removed"
    );
}

#[test]
fn read_chunk_row_returns_none_for_missing_chunk() {
    let (_tmp, cfg) = test_config();
    assert!(read_chunk_row(&cfg, "missing-chunk").unwrap().is_none());
}

#[test]
fn display_name_unslugs_email_thread_with_user_hint() {
    let name = display_name_for_source(
        "gmail:alice@example.com|bob@example.com",
        Some("alice@example.com"),
    );
    assert_eq!(name, "bob@example.com");
}

#[test]
fn display_name_falls_back_to_arrow_when_user_unknown() {
    let name = display_name_for_source("gmail:alice@example.com|bob@example.com", None);
    assert!(name.contains("alice@example.com"));
    assert!(name.contains("bob@example.com"));
    assert!(name.contains("↔"));
}

#[test]
fn display_name_strips_platform_prefix() {
    assert_eq!(
        display_name_for_source("slack:#engineering", None),
        "#engineering"
    );
}

#[test]
fn display_name_handles_multiple_participants_and_trimmed_hint() {
    let name = display_name_for_source(
        "gmail:Alice@Example.com|bob@example.com|carol@example.com",
        Some(" alice@example.com "),
    );
    assert_eq!(name, "bob@example.com, carol@example.com");
}

#[test]
fn display_name_handles_no_prefix() {
    assert_eq!(display_name_for_source("loose-id", None), "loose-id");
}

#[test]
fn sanitize_basename_replaces_windows_illegal_characters() {
    assert_eq!(
        sanitize_basename(r#"chat:slack/#eng\name*?"<>|"#),
        "chat-slack-#eng-name------"
    );
    assert_eq!(sanitize_basename("safe-name.md"), "safe-name.md");
}

#[test]
fn parse_source_kind_str_accepts_known_values_only() {
    assert_eq!(parse_source_kind_str("chat"), Some(SourceKind::Chat));
    assert_eq!(parse_source_kind_str("email"), Some(SourceKind::Email));
    assert_eq!(
        parse_source_kind_str("document"),
        Some(SourceKind::Document)
    );
    assert_eq!(parse_source_kind_str("unknown"), None);
}

#[test]
fn clear_composio_sync_state_removes_only_target_namespace() {
    let tmp = TempDir::new().unwrap();
    let _memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();
    let db_path = tmp.path().join("memory").join("memory.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();

    conn.execute(
        "INSERT INTO kv_namespace (namespace, key, value_json, updated_at)
         VALUES (?1, 'cursor', '{}', 1.0)",
        params![KV_NAMESPACE],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO kv_namespace (namespace, key, value_json, updated_at)
         VALUES ('other-namespace', 'cursor', '{}', 2.0)",
        [],
    )
    .unwrap();
    drop(conn);

    let removed = clear_composio_sync_state(&db_path).unwrap();
    assert_eq!(removed, 1);

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let composio_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM kv_namespace WHERE namespace = ?1",
            params![KV_NAMESPACE],
            |row| row.get(0),
        )
        .unwrap();
    let other_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM kv_namespace WHERE namespace = 'other-namespace'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(composio_count, 0);
    assert_eq!(other_count, 1);
}

// ── tree-mode graph export (summaries + leaf chunks) ────────────────────

/// Insert a tree row and one summary node under it.
fn insert_tree_summary(cfg: &Config, tree_id: &str, scope: &str, summary_id: &str, level: i64) {
    with_connection(cfg, |conn| {
        conn.execute(
            "INSERT OR IGNORE INTO mem_tree_trees (id, kind, scope, created_at_ms)
             VALUES (?1, 'source', ?2, 0)",
            params![tree_id, scope],
        )?;
        conn.execute(
            "INSERT INTO mem_tree_summaries (
                id, tree_id, tree_kind, level, child_ids_json, content, token_count,
                entities_json, topics_json, time_range_start_ms, time_range_end_ms,
                score, sealed_at_ms, deleted
             ) VALUES (?1, ?2, 'source', ?3, '[]', 'summary body', 1, '[]', '[]', 0, 0, 0.0, 0, 0)",
            params![summary_id, tree_id, level],
        )?;
        Ok(())
    })
    .unwrap();
}

/// Insert a leaf chunk, optionally linked to a parent summary.
fn insert_chunk_with_parent(
    cfg: &Config,
    id: &str,
    parent_summary_id: Option<&str>,
    timestamp_ms: i64,
    content: &str,
) {
    with_connection(cfg, |conn| {
        conn.execute(
            "INSERT INTO mem_tree_chunks (
                id, source_kind, source_id, source_ref, owner, timestamp_ms,
                time_range_start_ms, time_range_end_ms, tags_json, content,
                token_count, seq_in_source, created_at_ms, lifecycle_status,
                content_path, parent_summary_id
             ) VALUES (?1, 'chat', 'slack:#eng', NULL, 'tester', ?2, ?2, ?2, '[]', ?3, 1, 0, ?2, 'seeded', NULL, ?4)",
            params![id, timestamp_ms, content, parent_summary_id],
        )?;
        Ok(())
    })
    .unwrap();
}

#[tokio::test]
async fn tree_graph_includes_leaf_chunks_linked_to_their_summary() {
    let (_tmp, cfg) = test_config();
    insert_tree_summary(&cfg, "tree-1", "slack:#eng", "summary:1:L1-aaa", 1);
    insert_chunk_with_parent(
        &cfg,
        "chunk-sealed",
        Some("summary:1:L1-aaa"),
        1_700_000_000_000,
        "first line of sealed chunk\nmore body",
    );
    insert_chunk_with_parent(
        &cfg,
        "chunk-orphan",
        None,
        1_700_000_000_001,
        "orphan chunk body",
    );

    let resp = graph_export_rpc(&cfg, GraphMode::Tree).await.unwrap().value;

    // 1 source root + 1 summary + 2 leaf chunks = 4 nodes.
    assert_eq!(
        resp.nodes.len(),
        4,
        "source root + summary + both leaf chunks"
    );

    let source_root = resp.nodes.iter().find(|n| n.kind == "source").unwrap();
    assert!(source_root.id.starts_with("source:"));

    let summary = resp.nodes.iter().find(|n| n.kind == "summary").unwrap();
    assert_eq!(summary.id, "summary:1:L1-aaa");
    // Orphan summary links to source root.
    assert_eq!(summary.parent_id.as_deref(), Some(source_root.id.as_str()));

    let sealed = resp.nodes.iter().find(|n| n.id == "chunk-sealed").unwrap();
    assert_eq!(sealed.kind, "chunk");
    assert_eq!(sealed.parent_id.as_deref(), Some("summary:1:L1-aaa"));
    assert_eq!(sealed.label, "first line of sealed chunk");

    let orphan = resp.nodes.iter().find(|n| n.id == "chunk-orphan").unwrap();
    assert!(
        orphan.parent_id.is_none(),
        "unsealed chunk has no parent → renders as an orphan node"
    );

    assert!(resp.edges.is_empty());
}

#[tokio::test]
async fn tree_graph_keeps_summaries_first_then_chunks() {
    let (_tmp, cfg) = test_config();
    insert_tree_summary(&cfg, "tree-1", "slack:#eng", "summary:1:L1-aaa", 1);
    insert_chunk_with_parent(
        &cfg,
        "chunk-1",
        Some("summary:1:L1-aaa"),
        1_700_000_000_000,
        "a chunk",
    );

    let resp = graph_export_rpc(&cfg, GraphMode::Tree).await.unwrap().value;
    // Source roots are emitted first, then summaries, then chunks — so a
    // budget truncation drops chunk tails, never the tree skeleton.
    assert_eq!(resp.nodes[0].kind, "source");
    assert!(resp.nodes.iter().any(|n| n.kind == "summary"));
    assert!(resp.nodes.iter().any(|n| n.kind == "chunk"));
}

#[tokio::test]
async fn obsidian_status_registered_when_override_config_lists_content_root() {
    let (_tmp, cfg) = test_config();
    let content_root = cfg.memory_tree_content_root();
    // A separate dir standing in for a non-standard Obsidian config
    // location, with an obsidian.json that registers the content root.
    let cfg_dir = TempDir::new().unwrap();
    let body = format!(
        "{{ \"vaults\": {{ \"id0\": {{ \"path\": {}, \"open\": true }} }} }}",
        serde_json::to_string(&content_root.to_string_lossy().to_string()).unwrap()
    );
    std::fs::write(cfg_dir.path().join("obsidian.json"), body).unwrap();

    let outcome =
        obsidian_vault_status_rpc(&cfg, Some(cfg_dir.path().to_string_lossy().to_string()))
            .await
            .unwrap();

    assert!(outcome.value.registered);
    assert!(outcome.value.config_found);
    assert_eq!(
        outcome.value.content_root_abs,
        content_root.to_string_lossy().to_string()
    );
    // The log reports the booleans but redacts the absolute path (it
    // embeds the user's home / username).
    assert!(
        outcome.logs[0].contains("registered=true"),
        "log: {}",
        outcome.logs[0]
    );
    assert!(
        !outcome.logs[0].contains(content_root.to_str().unwrap()),
        "log leaked content root: {}",
        outcome.logs[0]
    );
}

#[tokio::test]
async fn obsidian_status_not_registered_for_empty_override_dir() {
    let (_tmp, cfg) = test_config();
    // Empty override dir → no obsidian.json there → content root is not a
    // registered vault. (A temp content root can't be under any real host
    // vault either, so this stays false regardless of the dev machine.)
    let cfg_dir = TempDir::new().unwrap();
    let outcome =
        obsidian_vault_status_rpc(&cfg, Some(cfg_dir.path().to_string_lossy().to_string()))
            .await
            .unwrap();
    assert!(!outcome.value.registered);
}

#[tokio::test]
async fn obsidian_status_blank_override_is_treated_as_none() {
    // A whitespace-only override must be normalized to None rather than
    // resolving to "." and probing a stray local ./obsidian.json. The temp
    // content root isn't under any real host vault, so this stays false.
    let (_tmp, cfg) = test_config();
    let outcome = obsidian_vault_status_rpc(&cfg, Some("   ".to_string()))
        .await
        .unwrap();
    assert!(!outcome.value.registered);
}

#[tokio::test]
async fn vault_health_check_reports_missing_content_root_for_fresh_workspace() {
    // `pipeline_healthy` reads the process-global degraded flags (via
    // `pipeline_status_rpc` → `current_degraded_state`), which sibling
    // `memory_tree` tests set and never clear. Serialise + reset to a clean
    // baseline so the assertion is deterministic. See #4691.
    let _g = crate::openhuman::memory_tree::health::test_guard();
    let (_tmp, cfg) = test_config();
    let outcome = vault_health_check_rpc(&cfg, None).await.unwrap();

    assert!(!outcome.value.exists);
    assert!(!outcome.value.readable);
    assert!(!outcome.value.writable);
    assert!(!outcome.value.obsidian_registered);
    assert!(outcome.value.pipeline_healthy);
    assert_eq!(outcome.value.last_sync_ms, 0);
}

/// #4278: both vault RPCs stamp the core host's OS so a frontend attached
/// from a different OS can tell `content_root_abs` is a foreign-host path and
/// must not open/reveal it locally.
#[tokio::test]
async fn vault_rpcs_report_core_host_os() {
    let (_tmp, cfg) = test_config();

    let status = obsidian_vault_status_rpc(&cfg, None).await.unwrap();
    assert_eq!(status.value.host_os, std::env::consts::OS);

    let health = vault_health_check_rpc(&cfg, None).await.unwrap();
    assert_eq!(health.value.host_os, std::env::consts::OS);
    assert!(
        !health.value.host_os.is_empty(),
        "host_os must be populated"
    );
}

#[tokio::test]
async fn vault_health_check_reports_writable_and_obsidian_registered_when_ready() {
    let (_tmp, cfg) = test_config();
    seed_chat_chunk(
        &cfg,
        "slack:#eng",
        "Vault health seed chunk so content_root exists and last_sync_ms > 0",
    )
    .await;

    let content_root = cfg.memory_tree_content_root();
    let cfg_dir = TempDir::new().unwrap();
    let body = format!(
        "{{ \"vaults\": {{ \"id0\": {{ \"path\": {}, \"open\": true }} }} }}",
        serde_json::to_string(&content_root.to_string_lossy().to_string()).unwrap()
    );
    std::fs::write(cfg_dir.path().join("obsidian.json"), body).unwrap();

    let outcome = vault_health_check_rpc(&cfg, Some(cfg_dir.path().to_string_lossy().to_string()))
        .await
        .unwrap();

    assert!(outcome.value.exists);
    assert!(outcome.value.readable);
    assert!(outcome.value.writable);
    assert!(outcome.value.obsidian_registered);
    // Intentionally NOT asserting `pipeline_healthy` here: with a seeded chunk
    // (total_chunks > 0) the derived status depends on the process-global
    // degraded flags, which unguarded parallel `memory_tree` extraction/pipeline
    // tests set and never clear (structure degrades only clears on a *successful*
    // extraction, which never happens under test). Post-#4691 a leaked "degraded"
    // correctly reads as unhealthy, so asserting healthy here would be flaky.
    // The health mapping is covered deterministically by the `pipeline_is_healthy`
    // unit tests in `read_rpc/vault.rs`; this test covers the filesystem readiness
    // wiring. See also `memory_tree::tree::rpc::pipeline_status_reports_chunk_aggregates_after_ingest`.
    assert!(outcome.value.last_sync_ms > 0);
    assert!(
        !outcome.logs[0].contains(content_root.to_str().unwrap()),
        "log leaked content root: {}",
        outcome.logs[0]
    );
}

/// Regression: `wipe_all` MUST also clear the source-ingest gate
/// (`mem_tree_ingested_sources`). Before the fix it cleared chunks/summaries
/// but left the gate claimed, so a wiped document source could never
/// re-ingest — the next sync saw `already_ingested` and wrote 0 chunks / 0
/// seal jobs. This pins that a wipe leaves the gate empty so re-sync works.
#[tokio::test]
async fn wipe_all_clears_ingest_gate() {
    use crate::openhuman::memory_store::chunks::store as chunk_store;
    use crate::openhuman::memory_store::chunks::types::SourceKind;

    let (_tmp, cfg) = test_config();
    let gate_key = "notion:conn-1:page-abc@1700000000000";

    // Claim the gate exactly as a document ingest does.
    chunk_store::with_connection(&cfg, |conn| {
        let tx = conn.unchecked_transaction()?;
        let claimed = chunk_store::claim_source_ingest_tx(
            &tx,
            SourceKind::Document,
            gate_key,
            1_700_000_000_000,
        )?;
        assert!(claimed, "first claim should succeed");
        tx.commit()?;
        Ok(())
    })
    .unwrap();
    assert!(
        chunk_store::is_source_ingested(&cfg, SourceKind::Document, gate_key).unwrap(),
        "gate must be claimed before wipe"
    );

    wipe_all_rpc(&cfg).await.expect("wipe_all_rpc");

    assert!(
        !chunk_store::is_source_ingested(&cfg, SourceKind::Document, gate_key).unwrap(),
        "wipe_all must clear mem_tree_ingested_sources so a wiped source can re-ingest"
    );
}
