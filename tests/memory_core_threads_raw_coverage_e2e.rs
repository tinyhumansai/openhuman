//! Round 16 raw integration coverage for memory-core and threads.
//!
//! These tests keep all state under temp workspaces and call public Rust
//! surfaces directly. Run with `--test-threads=1`; thread ops resolve the
//! workspace through process environment.

use chrono::{Duration, TimeZone, Utc};
use serde_json::json;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

use openhuman_core::openhuman::agent::progress::AgentProgress;
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::memory::read_rpc::{
    self, ChunkFilter, GraphMode, ResetTreeResponse,
};
use openhuman_core::openhuman::memory::tree_source::get_or_create_source_tree;
use openhuman_core::openhuman::memory::{
    AppendConversationMessageRequest, ConversationMessageRecord, ConversationMessagesRequest,
    CreateConversationThreadRequest, DeleteConversationThreadRequest, EmptyRequest,
    GenerateConversationThreadTitleRequest, UpdateConversationMessageRequest,
    UpdateConversationThreadLabelsRequest, UpdateConversationThreadTitleRequest,
};
use openhuman_core::openhuman::memory_conversations::{
    ensure_thread, list_threads, CreateConversationThread,
};
use openhuman_core::openhuman::memory_store::chunks::store::{upsert_chunks, with_connection};
use openhuman_core::openhuman::memory_store::chunks::types::{
    approx_token_count, chunk_id, Chunk, Metadata, SourceKind, SourceRef,
};
use openhuman_core::openhuman::memory_store::content;
use openhuman_core::openhuman::memory_store::trees::store as tree_store;
use openhuman_core::openhuman::memory_store::trees::types::{SummaryNode, TreeKind};
use openhuman_core::openhuman::memory_tree::score::embed::pack_embedding;
use openhuman_core::openhuman::memory_tree::score::extract::EntityKind;
use openhuman_core::openhuman::memory_tree::score::resolver::CanonicalEntity;
use openhuman_core::openhuman::memory_tree::score::signals::ScoreSignals;
use openhuman_core::openhuman::memory_tree::score::store::{index_entity, upsert_score, ScoreRow};
use openhuman_core::openhuman::threads::ops as thread_ops;
use openhuman_core::openhuman::threads::turn_state::{
    self, ClearTurnStateRequest, GetTurnStateRequest, TurnLifecycle, TurnStateMirror,
    TurnStateStore,
};
use openhuman_core::openhuman::threads::welcome_migration::migrate_welcome_agent_artifacts;

struct EnvGuard {
    key: &'static str,
    old: Option<OsString>,
}

impl EnvGuard {
    fn set_path(key: &'static str, value: &Path) -> Self {
        let old = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.old {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

fn config_in(tmp: &TempDir) -> Config {
    let mut cfg = Config {
        workspace_dir: tmp.path().to_path_buf(),
        embeddings_provider: Some("none".into()),
        memory_provider: Some("cloud".into()),
        ..Config::default()
    };
    cfg.memory_tree.embedding_endpoint = None;
    cfg.memory_tree.embedding_model = None;
    cfg.memory_tree.embedding_strict = false;
    cfg
}

fn test_chunk(source_id: &str, seq: u32, content: &str, ts_ms: i64) -> Chunk {
    let ts = Utc.timestamp_millis_opt(ts_ms).single().unwrap();
    let mut metadata = Metadata::point_in_time(SourceKind::Chat, source_id, "owner@example", ts);
    metadata.tags = vec!["round16".into(), format!("seq-{seq}")];
    metadata.source_ref = Some(SourceRef::new(format!("chat://{source_id}/{seq}")));
    Chunk {
        id: chunk_id(SourceKind::Chat, source_id, seq, content),
        content: content.to_string(),
        metadata,
        token_count: approx_token_count(content),
        seq_in_source: seq,
        created_at: ts,
        partial_message: false,
    }
}

fn seed_content_paths(cfg: &Config, chunks: &[Chunk]) {
    let root = cfg.memory_tree_content_root();
    fs::create_dir_all(&root).unwrap();
    let staged = content::stage_chunks(&root, chunks).unwrap();
    with_connection(cfg, |conn| {
        for staged_chunk in &staged {
            conn.execute(
                "UPDATE mem_tree_chunks
                    SET content_path = ?1, content_sha256 = ?2
                  WHERE id = ?3",
                rusqlite::params![
                    staged_chunk.content_path,
                    staged_chunk.content_sha256,
                    staged_chunk.chunk.id,
                ],
            )?;
        }
        Ok(())
    })
    .unwrap();
}

fn insert_summary(cfg: &Config, node: &SummaryNode, content_path: Option<&str>) {
    let embedding = node
        .embedding
        .as_ref()
        .map(|v| pack_embedding(v))
        .unwrap_or_default();
    with_connection(cfg, |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO mem_tree_summaries (
                id, tree_id, tree_kind, level, parent_id, child_ids_json,
                content, token_count, entities_json, topics_json,
                time_range_start_ms, time_range_end_ms, score, sealed_at_ms,
                deleted, embedding, content_path
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            rusqlite::params![
                node.id,
                node.tree_id,
                node.tree_kind.as_str(),
                node.level as i64,
                node.parent_id,
                serde_json::to_string(&node.child_ids)?,
                node.content,
                node.token_count as i64,
                serde_json::to_string(&node.entities)?,
                serde_json::to_string(&node.topics)?,
                node.time_range_start.timestamp_millis(),
                node.time_range_end.timestamp_millis(),
                node.score,
                node.sealed_at.timestamp_millis(),
                i32::from(node.deleted),
                if embedding.is_empty() {
                    None
                } else {
                    Some(embedding)
                },
                content_path,
            ],
        )?;
        Ok(())
    })
    .unwrap();
}

fn daily_node(id: &str, tree_id: &str, day: chrono::DateTime<Utc>) -> SummaryNode {
    SummaryNode {
        id: id.into(),
        tree_id: tree_id.into(),
        tree_kind: TreeKind::Global,
        level: 0,
        parent_id: None,
        child_ids: Vec::new(),
        content: format!("Daily digest for {id} with Alice and Phoenix planning."),
        token_count: 64,
        entities: vec!["person:alice".into()],
        topics: vec!["phoenix".into()],
        time_range_start: day,
        time_range_end: day + Duration::hours(1),
        score: 0.7,
        sealed_at: day + Duration::hours(2),
        deleted: false,
        embedding: Some(vec![0.0; 1024]),
        doc_id: None,
        version_ms: None,
    }
}

#[tokio::test]
async fn memory_read_rpc_filters_graphs_scores_reset_and_wipe_seeded_rows() {
    let tmp = TempDir::new().unwrap();
    let cfg = config_in(&tmp);
    let ts0 = Utc.with_ymd_and_hms(2026, 5, 20, 9, 0, 0).unwrap();
    let chunks = vec![
        test_chunk(
            "gmail:me@example.com|alice@example.com",
            0,
            "Alice shared the Phoenix launch checklist and budget.",
            ts0.timestamp_millis(),
        ),
        test_chunk(
            "slack:#ops",
            1,
            "Bob asked Alice for the deploy window in Phoenix.",
            (ts0 + Duration::hours(1)).timestamp_millis(),
        ),
    ];
    upsert_chunks(&cfg, &chunks).unwrap();
    seed_content_paths(&cfg, &chunks);

    let alice = CanonicalEntity {
        canonical_id: "person:alice".into(),
        kind: EntityKind::Person,
        surface: "Alice".into(),
        span_start: 0,
        span_end: 5,
        score: 0.95,
    };
    let topic = CanonicalEntity {
        canonical_id: "topic:phoenix".into(),
        kind: EntityKind::Topic,
        surface: "Phoenix".into(),
        span_start: 0,
        span_end: 7,
        score: 0.8,
    };
    for chunk in &chunks {
        index_entity(
            &cfg,
            &alice,
            &chunk.id,
            "leaf",
            chunk.metadata.timestamp.timestamp_millis(),
            Some("source:chat"),
        )
        .unwrap();
        index_entity(
            &cfg,
            &topic,
            &chunk.id,
            "leaf",
            chunk.metadata.timestamp.timestamp_millis(),
            Some("source:chat"),
        )
        .unwrap();
    }
    upsert_score(
        &cfg,
        &ScoreRow {
            chunk_id: chunks[0].id.clone(),
            total: 4.5,
            signals: ScoreSignals {
                token_count: 0.4,
                unique_words: 0.8,
                metadata_weight: 1.0,
                source_weight: 0.9,
                interaction: 0.7,
                entity_density: 0.6,
                llm_importance: 0.5,
            },
            dropped: false,
            reason: Some("coverage fixture".into()),
            computed_at_ms: ts0.timestamp_millis(),
            llm_importance_reason: Some("important planning".into()),
        },
    )
    .unwrap();

    let tree = get_or_create_source_tree(&cfg, "gmail:me@example.com|alice@example.com").unwrap();
    let summary = SummaryNode {
        id: "summary:L1:round16".into(),
        tree_id: tree.id.clone(),
        tree_kind: TreeKind::Source,
        level: 1,
        parent_id: None,
        child_ids: chunks.iter().map(|c| c.id.clone()).collect(),
        content: "Alice and Bob discussed Phoenix launch operations.".into(),
        token_count: 80,
        entities: vec!["person:alice".into()],
        topics: vec!["phoenix".into()],
        time_range_start: ts0,
        time_range_end: ts0 + Duration::hours(2),
        score: 0.9,
        sealed_at: ts0 + Duration::hours(3),
        deleted: false,
        embedding: Some(vec![0.0; 1024]),
        doc_id: None,
        version_ms: None,
    };
    insert_summary(
        &cfg,
        &summary,
        Some("wiki/summaries/source/summary-L1-round16.md"),
    );

    let listed = read_rpc::list_chunks_rpc(
        &cfg,
        ChunkFilter {
            source_kinds: Some(vec!["chat".into()]),
            entity_ids: Some(vec!["person:alice".into()]),
            query: Some("Phoenix".into()),
            limit: Some(10),
            ..ChunkFilter::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(listed.value.total, 2);
    assert!(listed.value.chunks[0].content_preview.is_some());

    let sources = read_rpc::list_sources_rpc(&cfg, Some("me@example.com".into()))
        .await
        .unwrap();
    assert!(sources.value.iter().any(|source| source.source_id
        == "gmail:me@example.com|alice@example.com"
        && source.chunk_count == 1));

    assert_eq!(
        read_rpc::search_rpc(&cfg, "deploy".into(), 5)
            .await
            .unwrap()
            .value
            .len(),
        1
    );
    assert_eq!(
        read_rpc::entity_index_for_rpc(&cfg, chunks[0].id.clone())
            .await
            .unwrap()
            .value
            .len(),
        2
    );
    assert_eq!(
        read_rpc::chunks_for_entity_rpc(&cfg, "person:alice".into())
            .await
            .unwrap()
            .value
            .len(),
        2
    );
    assert_eq!(
        read_rpc::top_entities_rpc(&cfg, Some("person".into()), 5)
            .await
            .unwrap()
            .value[0]
            .entity_id,
        "person:alice"
    );
    let score = read_rpc::chunk_score_rpc(&cfg, chunks[0].id.clone())
        .await
        .unwrap()
        .value
        .unwrap();
    assert!(score.kept);
    assert!(score.llm_consulted);

    let tree_graph = read_rpc::graph_export_rpc(&cfg, GraphMode::Tree)
        .await
        .unwrap();
    assert!(tree_graph
        .value
        .nodes
        .iter()
        .any(|node| node.kind == "summary"));
    let contacts_graph = read_rpc::graph_export_rpc(&cfg, GraphMode::Contacts)
        .await
        .unwrap();
    assert!(contacts_graph.value.edges.len() >= 2);
    assert!(
        !read_rpc::obsidian_vault_status_rpc(&cfg, Some("   ".into()))
            .await
            .unwrap()
            .value
            .registered
    );

    let deleted = read_rpc::delete_chunk_rpc(&cfg, chunks[1].id.clone())
        .await
        .unwrap();
    assert!(deleted.value.deleted);
    assert_eq!(deleted.value.entity_index_rows_removed, 2);
    assert!(
        !read_rpc::delete_chunk_rpc(&cfg, "missing-chunk".into())
            .await
            .unwrap()
            .value
            .deleted
    );

    let reset: ResetTreeResponse = read_rpc::reset_tree_rpc(&cfg).await.unwrap().value;
    assert!(reset.tree_rows_deleted >= 1);
    assert_eq!(reset.chunks_requeued, 1);
    assert_eq!(reset.jobs_enqueued, 1);
    let flush = read_rpc::flush_now_rpc(&cfg).await.unwrap().value;
    assert!(flush.enqueued);

    fs::create_dir_all(cfg.memory_tree_content_root().join("raw")).unwrap();
    fs::write(cfg.memory_tree_content_root().join("raw").join("x.md"), "x").unwrap();
    let wipe = read_rpc::wipe_all_rpc(&cfg).await.unwrap().value;
    assert!(wipe.rows_deleted >= 1);
    assert!(wipe.dirs_removed.iter().any(|dir| dir == "raw"));
}

#[tokio::test]
async fn thread_ops_welcome_migration_and_turn_state_cover_error_and_cleanup_paths() {
    let tmp = TempDir::new().unwrap();
    let _env = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let workspace = Config::load_or_init().await.unwrap().workspace_dir;

    ensure_thread(
        workspace.clone(),
        CreateConversationThread {
            id: "legacy-thread".into(),
            title: "Legacy".into(),
            created_at: "2026-05-01T00:00:00Z".into(),
            parent_thread_id: None,
            labels: Some(vec!["onboarding".into(), "inbox".into()]),
            personality_id: None,
        },
    )
    .unwrap();
    write_welcome_transcript(&workspace, "20260501_welcome", "welcome", "legacy-thread");
    fs::create_dir_all(workspace.join("sessions").join("legacy-thread")).unwrap();
    fs::write(
        workspace
            .join("sessions")
            .join("legacy-thread")
            .join("20260501_welcome.md"),
        "markdown",
    )
    .unwrap();

    let migration = migrate_welcome_agent_artifacts(&workspace).unwrap();
    assert_eq!(migration.threads_updated, 1);
    assert_eq!(migration.transcripts_updated, 1);
    assert_eq!(migration.transcript_files_renamed, 1);
    assert!(
        migrate_welcome_agent_artifacts(&workspace)
            .unwrap()
            .already_done
    );
    assert!(list_threads(workspace.clone())
        .unwrap()
        .into_iter()
        .find(|thread| thread.id == "legacy-thread")
        .unwrap()
        .labels
        .iter()
        .all(|label| label != "onboarding"));

    let created = thread_ops::thread_create_new(CreateConversationThreadRequest {
        labels: Some(vec!["chat".into()]),
        personality_id: Some("default".into()),
    })
    .await
    .unwrap()
    .value
    .data
    .unwrap();
    let thread_id = created.id;

    let msg_id = "msg-round16".to_string();
    let appended = thread_ops::message_append(AppendConversationMessageRequest {
        thread_id: thread_id.clone(),
        message: ConversationMessageRecord {
            id: msg_id.clone(),
            content: "Please summarize the Phoenix budget risks for Alice.".into(),
            message_type: "text".into(),
            extra_metadata: json!({"draft": true}),
            sender: "user".into(),
            created_at: "2026-05-21T10:00:00Z".into(),
        },
    })
    .await
    .unwrap()
    .value
    .data
    .unwrap();
    assert_eq!(appended.id, msg_id);

    let generated = thread_ops::thread_generate_title(GenerateConversationThreadTitleRequest {
        thread_id: thread_id.clone(),
        assistant_message: None,
    })
    .await
    .unwrap()
    .value
    .data
    .unwrap();
    assert!(generated.title.contains("Phoenix") || generated.title.contains("budget"));

    assert!(
        thread_ops::thread_update_title(UpdateConversationThreadTitleRequest {
            thread_id: thread_id.clone(),
            title: "   ".into(),
        })
        .await
        .is_err()
    );
    assert_eq!(
        thread_ops::thread_update_labels(UpdateConversationThreadLabelsRequest {
            thread_id: thread_id.clone(),
            labels: vec!["starred".into(), "archive".into()],
        })
        .await
        .unwrap()
        .value
        .data
        .unwrap()
        .labels,
        vec!["starred", "archive"]
    );
    let updated_msg = thread_ops::message_update(UpdateConversationMessageRequest {
        thread_id: thread_id.clone(),
        message_id: msg_id.clone(),
        extra_metadata: Some(json!({"draft": false, "edited": true})),
    })
    .await
    .unwrap()
    .value
    .data
    .unwrap();
    assert_eq!(updated_msg.extra_metadata["edited"], true);
    assert_eq!(
        thread_ops::messages_list(ConversationMessagesRequest {
            thread_id: thread_id.clone()
        })
        .await
        .unwrap()
        .value
        .data
        .unwrap()
        .count,
        1
    );

    let store = TurnStateStore::new(workspace.clone());
    let mut mirror = TurnStateMirror::new(store, &thread_id, "request-round16");
    assert!(!mirror.observe(&AgentProgress::ToolCallArgsDelta {
        call_id: "call-1".into(),
        tool_name: "memory.search".into(),
        delta: "{\"q\":\"phoenix\"}".into(),
        iteration: 1,
    }));
    assert!(mirror.observe(&AgentProgress::ToolCallStarted {
        call_id: "call-1".into(),
        tool_name: "memory.search".into(),
        arguments: json!({"q": "phoenix"}),
        iteration: 1,
        display_label: None,
        display_detail: None,
    }));
    assert!(mirror.observe(&AgentProgress::SubagentSpawned {
        agent_id: "researcher".into(),
        task_id: "task-1".into(),
        mode: "typed".into(),
        dedicated_thread: true,
        prompt_chars: 42,
        worker_thread_id: None,
        display_name: Some("Researcher".into()),
    }));
    assert!(mirror.observe(&AgentProgress::SubagentCompleted {
        agent_id: "researcher".into(),
        task_id: "task-1".into(),
        elapsed_ms: 50,
        iterations: 2,
        output_chars: 100,
        worktree_path: None,
        changed_files: vec![],
        dirty_status: None,
    }));
    mirror.finish();

    let turn_get = thread_ops::turn_state_get(GetTurnStateRequest {
        thread_id: thread_id.clone(),
    })
    .await
    .unwrap()
    .value
    .data
    .unwrap();
    assert_eq!(
        turn_get.turn_state.unwrap().lifecycle,
        TurnLifecycle::Interrupted
    );
    assert!(
        thread_ops::turn_state_clear(ClearTurnStateRequest {
            thread_id: thread_id.clone()
        })
        .await
        .unwrap()
        .value
        .data
        .unwrap()
        .cleared
    );
    assert!(turn_state::store::get(workspace.clone(), &thread_id)
        .unwrap()
        .is_none());

    let deleted = thread_ops::thread_delete(DeleteConversationThreadRequest {
        thread_id: thread_id.clone(),
        deleted_at: "2026-05-21T12:00:00Z".into(),
    })
    .await
    .unwrap()
    .value
    .data
    .unwrap();
    assert!(deleted.deleted);
    assert!(
        thread_ops::thread_generate_title(GenerateConversationThreadTitleRequest {
            thread_id,
            assistant_message: Some("unused".into()),
        })
        .await
        .is_err()
    );

    let purged = thread_ops::threads_purge(EmptyRequest {}).await.unwrap();
    assert_eq!(purged.value.data.unwrap().agent_threads_deleted, 1);
}

fn write_welcome_transcript(workspace: &Path, stem: &str, agent: &str, thread_id: &str) -> PathBuf {
    let path = workspace.join("session_raw").join(format!("{stem}.jsonl"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        format!(
            "{{\"_meta\":{{\"agent\":\"{agent}\",\"dispatcher\":\"native\",\"created\":\"2026-05-01T00:00:00Z\",\"updated\":\"2026-05-01T00:00:00Z\",\"turn_count\":1,\"input_tokens\":0,\"output_tokens\":0,\"cached_input_tokens\":0,\"charged_amount_usd\":0.0,\"thread_id\":\"{thread_id}\"}}}}\n{{\"role\":\"user\",\"content\":\"hi\"}}\n"
        ),
    )
    .unwrap();
    path
}
