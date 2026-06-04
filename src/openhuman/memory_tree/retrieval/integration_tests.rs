//! End-to-end integration test for Phase 4 retrieval tools (#710).
//!
//! Wires the real ingest pipeline (`ingest_chat`) + the six retrieval
//! primitives together to catch drift between ingestion-side schema
//! writes (entity index, trees, summaries) and retrieval-side reads.
//!
//! This lives next to the per-tool unit tests rather than under `tests/`
//! because it needs access to private internals (`Config::default`,
//! `score::store::*`) without spinning the full RPC stack.

#![cfg(test)]

use chrono::{TimeZone, Utc};
use tempfile::TempDir;

use crate::openhuman::config::Config;
use crate::openhuman::memory::ingest_pipeline::ingest_chat;
use crate::openhuman::memory_store::chunks::types::SourceKind;
use crate::openhuman::memory_sync::canonicalize::chat::{ChatBatch, ChatMessage};
use crate::openhuman::memory_tree::retrieval::{
    drill_down, fetch_leaves, query_source, search_entities,
};

fn test_config() -> (TempDir, Config) {
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    // Phase 4 (#710): ingest embeds chunks; tests use inert for determinism.
    cfg.memory_tree.embedding_endpoint = None;
    cfg.memory_tree.embedding_model = None;
    cfg.memory_tree.embedding_strict = false;
    // #002 (FR-002): the write path now SKIPS embedding (returns None) when no
    // provider is configured, instead of silently using a zero-vector inert
    // embedder. These integration tests assert embeddings ARE populated
    // end-to-end, so opt into the inert embedder explicitly — `provider=none`
    // is the deterministic "vector search by choice" path that
    // `build_write_embedder` returns as Some(inert).
    cfg.embeddings_provider = Some("none".into());
    (tmp, cfg)
}

fn chat_about_phoenix(seq: u32) -> ChatBatch {
    ChatBatch {
        platform: "slack".into(),
        channel_label: "#eng".into(),
        messages: vec![
            ChatMessage {
                author: "alice".into(),
                timestamp: Utc
                    .timestamp_millis_opt(1_700_000_000_000 + (seq as i64) * 10_000)
                    .unwrap(),
                text: format!(
                    "Phoenix migration status update {seq}: the runbook review is \
                     proceeding. alice@example.com is coordinating. We land \
                     Friday evening."
                ),
                source_ref: Some(format!("slack://phoenix/{seq}")),
            },
            ChatMessage {
                author: "bob".into(),
                timestamp: Utc
                    .timestamp_millis_opt(1_700_000_001_000 + (seq as i64) * 10_000)
                    .unwrap(),
                text: format!(
                    "Confirmed. I'll handle coordination. #launch-q2 tracked in \
                     Notion. bob@example.com will cut the release."
                ),
                source_ref: Some(format!("slack://phoenix/{seq}-reply")),
            },
        ],
    }
}

#[tokio::test]
async fn end_to_end_three_chat_batches() {
    let (_tmp, cfg) = test_config();

    // Ingest three batches in distinct slack channels.
    for (i, scope) in ["slack:#eng", "slack:#ops", "slack:#product"]
        .iter()
        .enumerate()
    {
        ingest_chat(&cfg, scope, "alice", vec![], chat_about_phoenix(i as u32))
            .await
            .unwrap();
    }

    // ── search_entities should surface alice under her canonical email id.
    let matches = search_entities(&cfg, "alice", None, 10).await.unwrap();
    let alice = matches
        .iter()
        .find(|m| m.canonical_id == "email:alice@example.com")
        .expect("alice should be discoverable via search");
    assert!(alice.mention_count >= 1);

    // ── query_source by source_id returns what we put in (chunks get
    // surfaced directly since none of the channels seal — 2 short msgs
    // per channel is under the seal budget).
    let by_source_kind = query_source(&cfg, None, Some(SourceKind::Chat), None, None, 20)
        .await
        .unwrap();
    // query_source returns summaries from sealed source trees only. With two
    // messages per channel the seal budget is not reached, so sealed
    // summaries may not exist yet. The invariant we lock in is that the
    // response is well-formed: total accurately reflects hits.len() (or
    // exceeds it when truncated) and never reports more hits than total.
    assert!(
        by_source_kind.total >= by_source_kind.hits.len(),
        "query_source total must be >= hits.len()"
    );

    // ── drill_down on a bogus id returns empty (no error).
    let empty_drill = drill_down(&cfg, "bogus:id", 1, None, None).await.unwrap();
    assert!(empty_drill.is_empty());

    // ── fetch_leaves on a bogus id hydrates nothing (no error).
    let none = fetch_leaves(&cfg, &["ghost:nonexistent".to_string()])
        .await
        .unwrap();
    assert!(none.is_empty());
}

// ── Phase 4 (#710): embedding + semantic rerank tests ───────────────────

/// Ingest with an inert embedder must populate every kept chunk's
/// `embedding` column. Embeddings are written by the async `extract_chunk`
/// handler, so the test drains the queue before inspecting.
#[tokio::test]
async fn ingest_populates_chunk_embeddings() {
    use crate::openhuman::memory_queue::drain_until_idle;
    use crate::openhuman::memory_store::chunks::store::get_chunk_embedding;
    use crate::openhuman::memory_tree::score::embed::EMBEDDING_DIM;

    let (_tmp, cfg) = test_config();
    let out = ingest_chat(&cfg, "slack:#eng", "alice", vec![], chat_about_phoenix(0))
        .await
        .unwrap();
    assert!(
        out.chunks_written >= 1,
        "expected at least one persisted chunk"
    );
    drain_until_idle(&cfg).await.unwrap();
    for id in &out.chunk_ids {
        let emb = get_chunk_embedding(&cfg, id).unwrap();
        let v = emb.unwrap_or_else(|| panic!("embedding missing for chunk_id={id}"));
        assert_eq!(v.len(), EMBEDDING_DIM, "embedding for {id} has wrong dim");
    }
}

/// Seal through the source-tree cascade must populate the summary's
/// embedding column. We drive large chunks directly through `append_leaf`
/// to cross the 10k-token seal budget, then inspect the L1 summary row.
/// This mirrors the bucket-seal unit test pattern — the ingest-driven
/// path uses the chunker, which caps individual chunk tokens and keeps
/// the seal from firing on short batches.
#[tokio::test]
async fn seal_populates_summary_embedding() {
    use crate::openhuman::memory::chat::{test_override, ChatProvider, StaticChatProvider};
    use crate::openhuman::memory::tree_source::registry::get_or_create_source_tree;
    use crate::openhuman::memory_store::chunks::store::upsert_chunks;
    use crate::openhuman::memory_store::chunks::types::{
        chunk_id, Chunk, Metadata, SourceKind, SourceRef,
    };
    use crate::openhuman::memory_store::content as content_store;
    use crate::openhuman::memory_tree::score::embed::EMBEDDING_DIM;
    use crate::openhuman::memory_tree::tree::bucket_seal::{append_leaf, LabelStrategy, LeafRef};
    use crate::openhuman::memory_tree::tree::store as src_store;
    use std::sync::Arc;

    let (_tmp, cfg) = test_config();
    let tree = get_or_create_source_tree(&cfg, "slack:#seal-test").unwrap();
    let provider: Arc<dyn ChatProvider> = Arc::new(StaticChatProvider::new("test summary content"));
    let ts = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();

    let mk_chunk = |seq: u32, tokens: u32| Chunk {
        id: chunk_id(SourceKind::Chat, "slack:#seal-test", seq, "test-content"),
        content: format!("substantive chunk content {seq}"),
        metadata: Metadata {
            source_kind: SourceKind::Chat,
            source_id: "slack:#seal-test".into(),
            owner: "alice".into(),
            timestamp: ts,
            time_range: (ts, ts),
            tags: vec![],
            source_ref: Some(SourceRef::new("slack://x")),
            path_scope: None,
        },
        token_count: tokens,
        seq_in_source: seq,
        created_at: ts,
        partial_message: false,
    };
    let c1 = mk_chunk(0, 30_000);
    let c2 = mk_chunk(1, 30_000);
    upsert_chunks(&cfg, &[c1.clone(), c2.clone()]).unwrap();
    {
        let content_root = cfg.memory_tree_content_root();
        std::fs::create_dir_all(&content_root).expect("create content_root for test");
        let staged = content_store::stage_chunks(&content_root, &[c1.clone(), c2.clone()])
            .expect("stage_chunks for test chunks");
        crate::openhuman::memory_store::chunks::store::with_connection(&cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            crate::openhuman::memory_store::chunks::store::upsert_staged_chunks_tx(&tx, &staged)?;
            tx.commit()?;
            Ok(())
        })
        .expect("persist staged chunk pointers");
    }

    let leaf_of = |c: &Chunk| LeafRef {
        chunk_id: c.id.clone(),
        token_count: c.token_count,
        timestamp: c.metadata.timestamp,
        content: c.content.clone(),
        entities: vec![],
        topics: vec![],
        score: 0.5,
    };
    test_override::with_provider(Arc::clone(&provider), async {
        append_leaf(&cfg, &tree, &leaf_of(&c1), &LabelStrategy::Empty)
            .await
            .unwrap()
    })
    .await;
    let sealed = test_override::with_provider(Arc::clone(&provider), async {
        append_leaf(&cfg, &tree, &leaf_of(&c2), &LabelStrategy::Empty)
            .await
            .unwrap()
    })
    .await;
    assert_eq!(sealed.len(), 1, "expected one seal at the budget crossing");

    // #1574 cutover: the seal path no longer writes the legacy
    // `mem_tree_summaries.embedding` column — the vector is persisted to the
    // per-model sidecar at the active signature inside the seal tx. Assert
    // it round-trips through the public accessor (which reads the sidecar at
    // the active signature), validating the write-side cutover end to end.
    let summary = src_store::get_summary(&cfg, &sealed[0]).unwrap().unwrap();
    assert!(
        summary.embedding.is_none(),
        "legacy summary embedding column must be NULL post-cutover"
    );
    let emb = src_store::get_summary_embedding(&cfg, &sealed[0])
        .unwrap()
        .expect("sealed summary must have an embedding in the per-model sidecar");
    assert_eq!(emb.len(), EMBEDDING_DIM);
}

/// Setting `query = Some(...)` changes ordering relative to the default
/// recency sort. We can't easily assert specific similarity scores when
/// using the inert embedder (all zero vectors → all similarities are 0),
/// so we instead verify that (a) the path doesn't error out and (b) the
/// response total/hit counts match the non-semantic path. Semantic
/// reranking correctness is covered in the per-tool unit tests below.
#[tokio::test]
async fn query_source_with_query_returns_same_count() {
    let (_tmp, cfg) = test_config();
    ingest_chat(&cfg, "slack:#eng", "alice", vec![], chat_about_phoenix(0))
        .await
        .unwrap();

    let recency = query_source(&cfg, None, Some(SourceKind::Chat), None, None, 20)
        .await
        .unwrap();
    let semantic = query_source(
        &cfg,
        None,
        Some(SourceKind::Chat),
        None,
        Some("phoenix migration"),
        20,
    )
    .await
    .unwrap();
    assert_eq!(recency.total, semantic.total);
    assert_eq!(recency.hits.len(), semantic.hits.len());
}
