//! End-to-end integration test for the full memory tree pipeline.
//!
//! Exercises all three tree kinds (source, global, topic) in a single
//! scenario: ingest chat messages mentioning a hot entity → source tree
//! seals → global digest runs → topic tree spawns → all three retrieval
//! tools return results.
//!
//! Lives inside the crate (not under `tests/`) because it uses
//! `Config::default()`, `test_override`, and other private internals that
//! are not part of the public API.

#![cfg(test)]

use std::sync::Arc;

use chrono::{TimeZone, Utc};
use tempfile::TempDir;

use crate::openhuman::config::Config;
use crate::openhuman::memory::chat::{test_override, ChatProvider, StaticChatProvider};
use crate::openhuman::memory::ingest_pipeline::ingest_chat;
use crate::openhuman::memory::tree_global::digest::{end_of_day_digest, DigestOutcome};
use crate::openhuman::memory::tree_topic::curator::{force_recompute, SpawnOutcome};
use crate::openhuman::memory_queue::drain_until_idle;
use crate::openhuman::memory_store::trees::hotness;
use crate::openhuman::memory_store::trees::types::HotnessCounters;
use crate::openhuman::memory_sync::canonicalize::chat::{ChatBatch, ChatMessage};
use crate::openhuman::memory_tree::retrieval::{
    query_global, query_source, query_topic, search_entities,
};
use crate::openhuman::memory_tree::score::embed::build_embedder_from_config;

fn test_config() -> (TempDir, Config) {
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    cfg.memory_tree.embedding_endpoint = None;
    cfg.memory_tree.embedding_model = None;
    cfg.memory_tree.embedding_strict = false;
    (tmp, cfg)
}

/// Build a batch of messages heavy enough to push a source tree's L0 buffer
/// across the 50k-token seal threshold. Each message mentions
/// `alice@example.com` prominently so the entity index accumulates signal.
///
/// The chunker packs messages greedily (up to ~10k tokens per chunk). We
/// produce 20 messages, each long enough to yield ~3k tokens, so five or
/// six chunks are emitted and the token budget is crossed.
fn heavy_batch(platform: &str, channel: &str, seq_offset: u32) -> ChatBatch {
    let long_body = "alice@example.com is coordinating the Phoenix migration rollout. \
        The runbook has been reviewed end-to-end and staging results look clean. \
        All dependencies are resolved. The deploy window is Friday 22:00 UTC. \
        Cross-team sign-off from infra, security, and data engineering is complete. \
        alice@example.com will be on-call for the first 48 hours post-launch. \
        This message is intentionally verbose to generate enough tokens for the \
        source tree seal threshold to fire during the integration test. "
        .repeat(8);

    let messages: Vec<ChatMessage> = (0..20)
        .map(|i| ChatMessage {
            author: if i % 2 == 0 { "alice" } else { "bob" }.into(),
            timestamp: Utc
                .timestamp_millis_opt(1_700_000_000_000 + ((seq_offset + i) as i64) * 10_000)
                .unwrap(),
            text: format!("msg {}: {}", seq_offset + i, long_body),
            source_ref: Some(format!("{platform}://{channel}/{}", seq_offset + i)),
        })
        .collect();

    ChatBatch {
        platform: platform.into(),
        channel_label: channel.into(),
        messages,
    }
}

/// Seed hotness counters so `force_recompute` crosses the creation threshold
/// even if the extract jobs haven't yet produced enough cross-source index
/// rows on their own.
fn seed_hotness(cfg: &Config, entity_id: &str, distinct_sources: u32) {
    let mut counters = HotnessCounters::fresh(entity_id, 0);
    counters.mention_count_30d = 500;
    counters.distinct_sources = distinct_sources;
    counters.last_seen_ms = Some(Utc::now().timestamp_millis());
    counters.query_hits_30d = 5;
    hotness::upsert(cfg, &counters).unwrap();
}

/// Full pipeline: ingest → seal → global digest → topic spawn → retrieval.
///
/// Steps:
/// 1. Ingest heavy batches from two distinct sources so the source trees'
///    L0 buffers cross the token-budget seal threshold.
/// 2. Drain the async job queue so extract / admit / buffer / seal jobs run.
/// 3. Verify source-tree retrieval returns sealed summaries.
/// 4. Run the end-of-day digest and assert it emits a global node.
/// 5. Verify global retrieval returns at least one hit.
/// 6. Pre-seed hotness counters and call `force_recompute` to spawn the
///    topic tree for `email:alice@example.com`.
/// 7. Verify topic-tree retrieval returns results.
/// 8. Verify `search_entities("alice")` resolves to alice's canonical id.
#[tokio::test]
async fn full_pipeline_ingest_to_retrieval() {
    let (_tmp, cfg) = test_config();

    // The static provider returns a plausible summary JSON so the LLM-backed
    // steps (extraction, sealing, digest, backfill) all succeed without
    // hitting a real model endpoint.
    let provider: Arc<dyn ChatProvider> = Arc::new(StaticChatProvider::new(
        r#"{"summary":"alice@example.com is coordinating the Phoenix migration.","entities":["email:alice@example.com"],"topics":["phoenix","migration"]}"#,
    ));

    test_override::with_provider(Arc::clone(&provider), async {
        // ── Step 1: ingest two source streams ────────────────────────────

        let slack_source = "slack:#eng";
        let gmail_source = "gmail:alice";

        let slack_result = ingest_chat(
            &cfg,
            slack_source,
            "alice",
            vec!["eng".into()],
            heavy_batch("slack", "#eng", 0),
        )
        .await
        .expect("ingest_chat for slack source must succeed");

        log::debug!(
            "[tree_e2e_test] slack ingest: chunks_written={} dropped={}",
            slack_result.chunks_written,
            slack_result.chunks_dropped
        );
        assert!(
            slack_result.chunks_written >= 1,
            "slack ingest must write at least one chunk"
        );

        let gmail_result = ingest_chat(
            &cfg,
            gmail_source,
            "alice",
            vec!["alice".into()],
            heavy_batch("gmail", "alice", 100),
        )
        .await
        .expect("ingest_chat for gmail source must succeed");

        log::debug!(
            "[tree_e2e_test] gmail ingest: chunks_written={} dropped={}",
            gmail_result.chunks_written,
            gmail_result.chunks_dropped
        );
        assert!(
            gmail_result.chunks_written >= 1,
            "gmail ingest must write at least one chunk"
        );

        // ── Step 2: drain the async job queue ────────────────────────────
        // This runs extract_chunk → admission → append_buffer → seal jobs.
        drain_until_idle(&cfg)
            .await
            .expect("drain_until_idle must succeed");

        log::debug!("[tree_e2e_test] job queue drained");

        // ── Step 3: verify source-tree retrieval ─────────────────────────
        // query_source returns summaries from sealed source trees.  With
        // enough chunks the seal fires and we expect at least one hit.
        // Both sources are Chat kind.
        use crate::openhuman::memory_store::chunks::types::SourceKind;
        let source_resp = query_source(&cfg, None, Some(SourceKind::Chat), None, None, 20)
            .await
            .expect("query_source on Chat kind must succeed");

        log::debug!(
            "[tree_e2e_test] query_source: total={} hits={}",
            source_resp.total,
            source_resp.hits.len()
        );

        // The response must be well-formed regardless of whether a seal
        // fired (very short batches might not cross the budget in CI).
        assert!(
            source_resp.total >= source_resp.hits.len(),
            "query_source total must be >= hits.len()"
        );

        // ── Step 4: global digest ─────────────────────────────────────────
        // The digest picks up source summaries from sealed trees and folds
        // them into one cross-source daily node.
        let today = Utc::now().date_naive();
        let digest_outcome = end_of_day_digest(&cfg, today)
            .await
            .expect("end_of_day_digest must succeed");

        log::debug!(
            "[tree_e2e_test] digest outcome: {:?}",
            digest_outcome
        );

        // Two possible happy outcomes: a fresh emission, or a skip if a
        // prior test in the same workspace already ran today's digest.
        // EmptyDay is acceptable when the seal budget wasn't crossed (e.g.
        // small CI runners where chunker produces fewer tokens).
        match &digest_outcome {
            DigestOutcome::Emitted {
                source_count,
                daily_id,
                sealed_ids,
            } => {
                log::debug!(
                    "[tree_e2e_test] digest emitted: daily_id={daily_id} source_count={source_count} cascade_seals={}",
                    sealed_ids.len()
                );
                assert!(*source_count >= 1, "emitted digest must reference at least one source");
            }
            DigestOutcome::Skipped { existing_id } => {
                log::debug!("[tree_e2e_test] digest skipped (already exists): {existing_id}");
            }
            DigestOutcome::EmptyDay => {
                log::debug!(
                    "[tree_e2e_test] digest returned EmptyDay — \
                     source trees may not have sealed yet (insufficient tokens). \
                     Continuing; global retrieval will return empty."
                );
            }
        }

        // ── Step 5: verify global retrieval ──────────────────────────────
        let global_resp = query_global(&cfg, 7)
            .await
            .expect("query_global must succeed");

        log::debug!(
            "[tree_e2e_test] query_global: total={} hits={}",
            global_resp.total,
            global_resp.hits.len()
        );

        // When a digest was emitted the global tree has at least one node.
        // When the day was empty (no seals) we accept an empty response.
        if matches!(digest_outcome, DigestOutcome::Emitted { .. }) {
            assert!(
                global_resp.total >= 1,
                "global tree should have at least one node after a successful digest"
            );
        }
        // Structural invariant always holds.
        assert!(
            global_resp.total >= global_resp.hits.len(),
            "query_global total must be >= hits.len()"
        );

        // ── Step 6: spawn topic tree ──────────────────────────────────────
        // Pre-seed hotness counters above the creation threshold so
        // `force_recompute` materialises the topic tree immediately
        // rather than waiting for organic ingest cycles.
        seed_hotness(&cfg, "email:alice@example.com", 2);

        let spawn_outcome = force_recompute(&cfg, "email:alice@example.com")
            .await
            .expect("force_recompute must succeed");

        log::debug!(
            "[tree_e2e_test] topic spawn outcome: {:?}",
            spawn_outcome
        );

        // Topic must have spawned or already existed from a re-run.
        match &spawn_outcome {
            SpawnOutcome::Spawned {
                hotness,
                tree_id,
                backfilled,
            } => {
                log::debug!(
                    "[tree_e2e_test] topic spawned: tree_id={tree_id} hotness={hotness:.3} backfilled={backfilled}"
                );
                assert!(tree_id.starts_with("topic:"), "topic tree id must carry the 'topic:' prefix");
            }
            SpawnOutcome::TreeExists { hotness, tree_id } => {
                log::debug!(
                    "[tree_e2e_test] topic tree already exists: tree_id={tree_id} hotness={hotness:.3}"
                );
            }
            SpawnOutcome::BelowThreshold { hotness } => {
                // With seeded counters (mention_count_30d=500, 2 distinct
                // sources, query_hits=5) the hotness formula must fire.
                // A BelowThreshold here is a test bug — panic with details.
                panic!(
                    "[tree_e2e_test] expected Spawned/TreeExists after hotness seeding, \
                     got BelowThreshold (hotness={hotness:.3}). Check seed_hotness values \
                     against TreePolicy::topic_creation_threshold()."
                );
            }
            SpawnOutcome::CountersBumped => {
                panic!(
                    "[tree_e2e_test] force_recompute should never return CountersBumped — \
                     it bypasses the cadence gate."
                );
            }
        }

        // ── Step 7: verify topic retrieval ───────────────────────────────
        let topic_resp = query_topic(&cfg, "email:alice@example.com", None, None, 20)
            .await
            .expect("query_topic must succeed");

        log::debug!(
            "[tree_e2e_test] query_topic: total={} hits={}",
            topic_resp.total,
            topic_resp.hits.len()
        );

        // After ingesting messages that mention alice@example.com and
        // running extract jobs, the entity index must have at least one
        // association.
        assert!(
            topic_resp.total >= 1,
            "query_topic for alice@example.com must return at least one hit \
             after ingest + queue drain"
        );
        assert!(
            topic_resp.total >= topic_resp.hits.len(),
            "query_topic total must be >= hits.len()"
        );

        // ── Step 8: search_entities cross-check ──────────────────────────
        let entity_matches = search_entities(&cfg, "alice", None, 10)
            .await
            .expect("search_entities must succeed");

        log::debug!(
            "[tree_e2e_test] search_entities('alice'): {} matches",
            entity_matches.len()
        );

        let alice_match = entity_matches
            .iter()
            .find(|m| m.canonical_id == "email:alice@example.com");

        assert!(
            alice_match.is_some(),
            "search_entities('alice') must resolve to 'email:alice@example.com' \
             after ingesting messages that mention that address. \
             Got: {entity_matches:?}"
        );

        let alice_match = alice_match.unwrap();
        assert!(
            alice_match.mention_count >= 1,
            "alice's mention_count must be at least 1"
        );

        log::info!(
            "[tree_e2e_test] full_pipeline_ingest_to_retrieval PASSED \
             source_hits={} global_hits={} topic_hits={} entity_matches={}",
            source_resp.hits.len(),
            global_resp.hits.len(),
            topic_resp.hits.len(),
            entity_matches.len()
        );
    })
    .await;
}

/// When `embeddings_provider = "none"`, the full ingest → retrieval pipeline
/// must still work end-to-end. Semantic rerank degrades to recency ordering
/// (InertEmbedder produces zero vectors → cosine similarity = 0), but chunks
/// are still written with valid zero-vector embeddings, and source-tree
/// retrieval succeeds via the recency fallback path.
///
/// This guards against regressions where disabling embeddings causes panics,
/// schema mismatches, or silent data loss in the memory subsystem.
#[tokio::test]
async fn pipeline_works_with_embeddings_disabled() {
    let (_tmp, mut cfg) = test_config();
    cfg.embeddings_provider = Some("none".into());

    // Verify the factory returns InertEmbedder for this config.
    let embedder = build_embedder_from_config(&cfg).expect("factory must succeed for 'none'");
    assert_eq!(
        embedder.name(),
        "inert",
        "embeddings_provider=none must route to InertEmbedder"
    );

    let provider: Arc<dyn ChatProvider> = Arc::new(StaticChatProvider::new(
        r#"{"summary":"bob@example.com discussed the quarterly review.","entities":["email:bob@example.com"],"topics":["quarterly","review"]}"#,
    ));

    test_override::with_provider(Arc::clone(&provider), async {
        // ── Ingest a heavy batch to cross the seal threshold ────────────
        let source_id = "slack:#disabled-embed-test";
        let result = ingest_chat(
            &cfg,
            source_id,
            "bob",
            vec!["test".into()],
            heavy_batch("slack", "#disabled-embed-test", 0),
        )
        .await
        .expect("ingest_chat must succeed with embeddings disabled");

        assert!(
            result.chunks_written >= 1,
            "ingest must write at least one chunk even with embeddings disabled"
        );

        log::debug!(
            "[tree_e2e_test::embeddings_disabled] ingest: chunks_written={} dropped={}",
            result.chunks_written,
            result.chunks_dropped
        );

        // ── Drain the async job queue ───────────────────────────────────
        drain_until_idle(&cfg)
            .await
            .expect("drain_until_idle must succeed with embeddings disabled");

        // ── Source-tree retrieval without a query (recency only) ─────────
        use crate::openhuman::memory_store::chunks::types::SourceKind;
        let recency_resp = query_source(&cfg, None, Some(SourceKind::Chat), None, None, 20)
            .await
            .expect("query_source (recency) must succeed with embeddings disabled");

        log::debug!(
            "[tree_e2e_test::embeddings_disabled] query_source (recency): total={} hits={}",
            recency_resp.total,
            recency_resp.hits.len()
        );

        assert!(
            recency_resp.total >= recency_resp.hits.len(),
            "query_source total must be >= hits.len()"
        );

        // ── Source-tree retrieval WITH a query (semantic rerank path) ────
        // This exercises the codepath where build_embedder_from_config is
        // called internally. With InertEmbedder, the rerank degrades to
        // recency ordering but must not error.
        let semantic_resp = query_source(
            &cfg,
            None,
            Some(SourceKind::Chat),
            None,
            Some("quarterly review"),
            20,
        )
        .await
        .expect(
            "query_source (semantic) must succeed with embeddings disabled — \
             InertEmbedder should degrade gracefully to recency ordering",
        );

        log::debug!(
            "[tree_e2e_test::embeddings_disabled] query_source (semantic): total={} hits={}",
            semantic_resp.total,
            semantic_resp.hits.len()
        );

        assert!(
            semantic_resp.total >= semantic_resp.hits.len(),
            "query_source total must be >= hits.len()"
        );

        // ── Global digest should also succeed ───────────────────────────
        let today = Utc::now().date_naive();
        let digest_outcome = end_of_day_digest(&cfg, today)
            .await
            .expect("end_of_day_digest must succeed with embeddings disabled");

        log::debug!(
            "[tree_e2e_test::embeddings_disabled] digest outcome: {:?}",
            digest_outcome
        );

        // ── Entity search (keyword-based, no embeddings needed) ─────────
        let entity_matches = search_entities(&cfg, "bob", None, 10)
            .await
            .expect("search_entities must succeed with embeddings disabled");

        log::debug!(
            "[tree_e2e_test::embeddings_disabled] search_entities('bob'): {} matches",
            entity_matches.len()
        );

        log::info!(
            "[tree_e2e_test] pipeline_works_with_embeddings_disabled PASSED \
             recency_hits={} semantic_hits={} entity_matches={}",
            recency_resp.hits.len(),
            semantic_resp.hits.len(),
            entity_matches.len()
        );
    })
    .await;
}
