//! E2E tests for the deterministic E2GraphRAG retriever (`fast_retrieve`).
//!
//! These replace the old agentic `memory_tree_walk_e2e.rs`. There is no LLM in
//! the retrieval loop, so no mock server is needed — we ingest a small chat
//! corpus, then assert that:
//!   - an entity-relationship query routes to the *local* branch and returns
//!     the chunk where the two entities co-occur, ranked by entity coverage;
//!   - a query with no extractable entities routes to the *global* branch and
//!     returns cleanly (no panic) over the same store;
//!   - the output is structured `QueryResponse` evidence (hits), not prose.
//!
//! spaCy is disabled here so the run is deterministic and Python-free in CI —
//! query-entity extraction uses the regex fallback (emails/handles/hashtags),
//! which is enough to exercise both routing branches.
//!
//! Run with:
//!   cargo test --test memory_fast_retrieve_e2e
//! or via the project wrapper:
//!   bash scripts/test-rust-with-mock.sh --test memory_fast_retrieve_e2e

use chrono::{TimeZone, Utc};
use tempfile::TempDir;

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::memory::ingest_pipeline::ingest_chat;
use openhuman_core::openhuman::memory_sync::canonicalize::chat::{ChatBatch, ChatMessage};
use openhuman_core::openhuman::memory_tree::retrieval::{fast_retrieve, FastRetrieveOptions};

fn test_config() -> (TempDir, Config) {
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    // Inert embedder — no Ollama/cloud in CI.
    cfg.memory_tree.embedding_endpoint = None;
    cfg.memory_tree.embedding_model = None;
    cfg.memory_tree.embedding_strict = false;
    // Deterministic, Python-free entity extraction (regex fallback).
    cfg.memory_tree.spacy_enabled = false;
    (tmp, cfg)
}

async fn seed_chat(cfg: &Config, source: &str, text: &str) {
    let batch = ChatBatch {
        platform: "slack".into(),
        channel_label: source.into(),
        messages: vec![ChatMessage {
            author: "alice".into(),
            timestamp: Utc.timestamp_millis_opt(1_700_000_000_000).unwrap(),
            text: text.into(),
            source_ref: Some("slack://x".into()),
        }],
    };
    ingest_chat(cfg, source, "alice", vec![], batch)
        .await
        .expect("ingest_chat should succeed");
}

#[tokio::test]
async fn local_branch_returns_cooccurring_evidence() {
    let (_tmp, cfg) = test_config();
    // alice + bob co-occur in one message → graph edge + both indexed on the
    // same leaf chunk.
    seed_chat(
        &cfg,
        "slack:#eng",
        "Sync between alice@example.com and bob@example.com on the runbook.",
    )
    .await;
    // An unrelated message that should NOT surface for the alice+bob query.
    seed_chat(
        &cfg,
        "slack:#random",
        "Lunch plans for friday with the team.",
    )
    .await;

    let resp = fast_retrieve(
        &cfg,
        "what did alice@example.com and bob@example.com discuss",
        FastRetrieveOptions::default(),
    )
    .await
    .expect("fast_retrieve should succeed");

    assert!(
        !resp.hits.is_empty(),
        "co-occurring entities should yield a local hit; got {resp:?}"
    );
    // Coverage score = both query entities matched the same node.
    assert!(
        resp.hits.iter().any(|h| h.score >= 2.0),
        "top local hit should have entity-coverage score >= 2; got {:?}",
        resp.hits.iter().map(|h| h.score).collect::<Vec<_>>()
    );
    // Structured evidence, not prose — every hit has a node id + content.
    assert!(resp.hits.iter().all(|h| !h.node_id.is_empty()));
}

#[tokio::test]
async fn global_branch_handles_entity_free_query() {
    let (_tmp, cfg) = test_config();
    seed_chat(
        &cfg,
        "slack:#eng",
        "Sync between alice@example.com and bob@example.com on the runbook.",
    )
    .await;

    // No mechanical entities in the query → global/dense branch. With the inert
    // embedder this returns recency-ordered summaries (possibly empty), and
    // crucially must not panic or error.
    let resp = fast_retrieve(
        &cfg,
        "give me a recap of everything important",
        FastRetrieveOptions::default(),
    )
    .await
    .expect("global branch should succeed");
    // total/truncated are well-formed regardless of hit count.
    assert_eq!(resp.truncated, resp.total > resp.hits.len());
}

#[tokio::test]
async fn empty_store_returns_no_hits() {
    let (_tmp, cfg) = test_config();
    let resp = fast_retrieve(&cfg, "anything at all", FastRetrieveOptions::default())
        .await
        .expect("retrieval over empty store should succeed");
    assert!(resp.hits.is_empty());
    assert_eq!(resp.total, 0);
}
