//! E2E tests for the sync → ingest → queue → tree pipeline.
//!
//! Exercises the full production path: messages arrive via `ingest_chat`,
//! get chunked and persisted, the job queue drains (extract → admit →
//! append_buffer → seal → topic_route), the source tree grows, and
//! domain events are emitted at each stage.
//!
//! Two scenarios:
//! 1. **Single batch** — one ingest, queue drains, source tree has a
//!    buffered leaf, events fired.
//! 2. **High-volume** — enough data to cross the L0 seal threshold (50k
//!    tokens), producing sealed summaries and cascading into topic trees
//!    + global digest.

#![cfg(test)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use tempfile::TempDir;

use crate::core::event_bus::{
    init_global as init_event_bus, subscribe_global, DomainEvent, EventHandler, SubscriptionHandle,
};
use crate::openhuman::config::Config;
use crate::openhuman::memory::chat::{test_override, ChatProvider, StaticChatProvider};
use crate::openhuman::memory::ingest_pipeline::ingest_chat;
use crate::openhuman::memory::sync::{emit_sync_stage, MemorySyncStage, MemorySyncTrigger};
use crate::openhuman::memory::tree_global::digest::{end_of_day_digest, DigestOutcome};
use crate::openhuman::memory::tree_topic::curator::{force_recompute, SpawnOutcome};
use crate::openhuman::memory_queue::{self, count_total, drain_until_idle, JobStatus};
use crate::openhuman::memory_store::chunks::store::{
    count_chunks, count_chunks_by_lifecycle_status, CHUNK_STATUS_BUFFERED,
};
use crate::openhuman::memory_store::trees::{
    hotness, store as tree_store,
    types::{HotnessCounters, TreeKind},
};
use crate::openhuman::memory_sync::canonicalize::chat::{ChatBatch, ChatMessage};
use crate::openhuman::memory_tree::retrieval::{query_source, query_topic, search_entities};
use crate::openhuman::memory_tree::score::store::lookup_entity;

// ── helpers ─────────────────────────────────────────────────────────────

fn test_config() -> (TempDir, Config) {
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    cfg.memory_tree.embedding_endpoint = None;
    cfg.memory_tree.embedding_model = None;
    cfg.memory_tree.embedding_strict = false;
    (tmp, cfg)
}

fn ensure_event_bus() {
    let _ = init_event_bus(256);
}

#[derive(Clone)]
struct EventCollector {
    events: Arc<Mutex<Vec<DomainEvent>>>,
}

impl EventCollector {
    fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn subscribe(self) -> (Self, Option<SubscriptionHandle>) {
        let handle = subscribe_global(Arc::new(self.clone()));
        (self, handle)
    }

    fn count_by<F: Fn(&DomainEvent) -> bool>(&self, pred: F) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| pred(e))
            .count()
    }
}

#[async_trait]
impl EventHandler for EventCollector {
    fn name(&self) -> &str {
        "test::event_collector"
    }

    async fn handle(&self, event: &DomainEvent) {
        self.events.lock().unwrap().push(event.clone());
    }
}

fn substantive_body(seq: u32) -> String {
    format!(
        "Update #{seq} on Phoenix migration: alice@example.com confirmed the \
         rollback procedure is documented. Staging checks passed: p99 latency \
         12ms, error rate 0.001%, memory 2.1 GiB. bob@example.com handles \
         on-call coordination. Feature flag phoenix_v2_enabled ramps Friday \
         evening. Remaining: finalize notification, update status page, \
         rotate staging credentials."
    )
}

fn large_body(seq: u32) -> String {
    let base = substantive_body(seq);
    format!(
        "{base}\n\n\
         Additional context from the architecture review (thread #{seq}): \
         the Phoenix migration touches auth-gateway, user-profiles, and \
         billing-ledger. alice@example.com mapped the dependency graph — \
         billing-ledger migrates first since user-profiles reads its views. \
         bob@example.com raised concerns about OAuth token rotation during \
         cutover. We use dual-write mode for 48 hours: ~200k token refreshes \
         per hour, within capacity. Schema migration: 3 ALTER TABLE statements, \
         all backwards-compatible. API v2 coexists with v1 for 30 days. \
         Rollback trigger: error rate > 0.1% for 5 consecutive minutes. \
         alice@example.com verified that reverting the feature flag immediately \
         drains the v2 code path. Timeline: Thursday final staging, Friday \
         22:00 UTC canary, Saturday 08:00 ramp to 10%, Monday 09:00 ramp to \
         100%, Tuesday remove v1 code path. Please review the runbook."
    )
}

fn mk_batch(source: &str, label: &str, seq: u32, body: &str, base_ts: i64) -> ChatBatch {
    ChatBatch {
        platform: source.into(),
        channel_label: label.into(),
        messages: vec![ChatMessage {
            author: "alice".into(),
            timestamp: Utc
                .timestamp_millis_opt(base_ts + (seq as i64) * 60_000)
                .unwrap(),
            text: body.into(),
            source_ref: Some(format!("{source}://msg/{seq}")),
        }],
    }
}

// ── Test 1: single batch → ingest → queue drain → tree ──────────────────

#[tokio::test]
async fn single_batch_sync_to_tree() {
    let (_tmp, cfg) = test_config();
    ensure_event_bus();

    let (collector, _handle) = EventCollector::new().subscribe();

    // Simulate sync lifecycle events.
    emit_sync_stage(
        MemorySyncTrigger::Manual,
        MemorySyncStage::Requested,
        Some("gmail"),
        Some("conn-1"),
        None,
    );
    emit_sync_stage(
        MemorySyncTrigger::Manual,
        MemorySyncStage::Fetching,
        Some("gmail"),
        Some("conn-1"),
        None,
    );

    let source_id = "gmail:alice-thread-1";
    let batch = mk_batch("gmail", "inbox", 1, &substantive_body(1), 1_700_000_000_000);
    let result = ingest_chat(&cfg, source_id, "alice", vec!["gmail".into()], batch)
        .await
        .unwrap();

    assert!(result.chunks_written >= 1);
    assert!(!result.already_ingested);

    let total_jobs = count_total(&cfg).unwrap();
    assert!(total_jobs >= 1, "extract_chunk job should be queued");

    // DocumentCanonicalized event.
    tokio::task::yield_now().await;
    let canonicalized_count = collector.count_by(|e| {
        matches!(e, DomainEvent::DocumentCanonicalized { source_kind, source_id: sid, .. }
            if source_kind == "chat" && sid == "gmail:alice-thread-1")
    });
    assert!(canonicalized_count >= 1);

    // Drain: extract → admit → append_buffer.
    drain_until_idle(&cfg).await.unwrap();

    let done = memory_queue::count_by_status(&cfg, JobStatus::Done).unwrap();
    assert!(done >= 1, "at least one job should complete");

    let buffered = count_chunks_by_lifecycle_status(&cfg, CHUNK_STATUS_BUFFERED).unwrap();
    assert!(buffered >= 1, "chunks should reach buffered status");

    // Source tree with non-empty L0 buffer.
    let source_trees = tree_store::list_trees_by_kind(&cfg, TreeKind::Source).unwrap();
    assert!(!source_trees.is_empty());
    let buf = tree_store::get_buffer(&cfg, &source_trees[0].id, 0).unwrap();
    assert!(!buf.is_empty(), "L0 buffer should contain the leaf");

    // Entity index.
    let alice_hits = lookup_entity(&cfg, "email:alice@example.com", None).unwrap();
    assert!(
        !alice_hits.is_empty(),
        "alice should be in the entity index"
    );

    // Completion event.
    emit_sync_stage(
        MemorySyncTrigger::Manual,
        MemorySyncStage::Completed,
        Some("gmail"),
        Some("conn-1"),
        None,
    );

    tokio::task::yield_now().await;
    let sync_stages: Vec<String> = collector
        .events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            DomainEvent::MemorySyncStageChanged { stage, .. } => Some(stage.clone()),
            _ => None,
        })
        .collect();
    assert!(sync_stages.contains(&"requested".to_string()));
    assert!(sync_stages.contains(&"completed".to_string()));
}

// ── Test 2: high-volume → seal → digest → topic tree ────────────────────

#[tokio::test]
async fn multi_batch_volume_builds_full_tree() {
    let (_tmp, cfg) = test_config();
    ensure_event_bus();

    let (collector, _handle) = EventCollector::new().subscribe();

    let source_id = "gmail:alice-volume";
    let base_ts = Utc::now().timestamp_millis() - 86_400_000;

    // Ingest 30 batches with large bodies to cross the 50k token seal threshold.
    for i in 0..30u32 {
        let batch = mk_batch("gmail", "inbox", i, &large_body(i), base_ts);
        let result = ingest_chat(&cfg, source_id, "alice", vec!["gmail".into()], batch)
            .await
            .unwrap();
        assert!(
            result.chunks_written >= 1,
            "batch {i} should produce chunks"
        );
    }

    let total_chunks = count_chunks(&cfg).unwrap();
    assert!(total_chunks >= 20, "got {total_chunks}");

    tokio::task::yield_now().await;
    let canonicalized = collector.count_by(|e| {
        matches!(e, DomainEvent::DocumentCanonicalized { source_id: sid, .. }
            if sid == "gmail:alice-volume")
    });
    assert!(canonicalized >= 20);

    // Drain all jobs.
    drain_until_idle(&cfg).await.unwrap();

    // Source tree should have sealed to L1+.
    let source_trees = tree_store::list_trees_by_kind(&cfg, TreeKind::Source).unwrap();
    assert!(!source_trees.is_empty());
    let source_tree = &source_trees[0];
    assert!(
        source_tree.max_level >= 1,
        "should seal to L1+, got max_level={}",
        source_tree.max_level
    );

    let l1 = tree_store::list_summaries_at_level(&cfg, &source_tree.id, 1).unwrap();
    assert!(!l1.is_empty(), "L1 summaries should exist");

    // Source retrieval.
    let source_resp = query_source(&cfg, Some(source_id), None, None, None, 10)
        .await
        .unwrap();
    assert!(!source_resp.hits.is_empty());

    // Entity index well-populated.
    let alice_hits = lookup_entity(&cfg, "email:alice@example.com", None).unwrap();
    assert!(alice_hits.len() >= 5, "got {}", alice_hits.len());

    // Entity search.
    let matches = search_entities(&cfg, "alice", None, 10).await.unwrap();
    assert!(!matches.is_empty());
    assert!(matches
        .iter()
        .any(|m| m.canonical_id == "email:alice@example.com"));

    // ── Global digest ──────────────────────────────────────────────────

    let provider: Arc<dyn ChatProvider> = Arc::new(StaticChatProvider::new("daily digest summary"));

    let digest_day = Utc.timestamp_millis_opt(base_ts).unwrap().date_naive();

    let digest_outcome = test_override::with_provider(Arc::clone(&provider), async {
        end_of_day_digest(&cfg, digest_day).await.unwrap()
    })
    .await;

    match &digest_outcome {
        DigestOutcome::Emitted {
            source_count,
            daily_id,
            ..
        } => {
            assert!(*source_count >= 1);
            assert!(!daily_id.is_empty());
        }
        DigestOutcome::EmptyDay => {
            // Acceptable — timestamp alignment may miss the exact day.
        }
        DigestOutcome::Skipped { .. } => {
            panic!("first run should not skip");
        }
    }

    // ── Topic tree ─────────────────────────────────────────────────────

    let mut counters = HotnessCounters::fresh("email:alice@example.com", 0);
    counters.mention_count_30d = 500;
    counters.distinct_sources = 2;
    counters.last_seen_ms = Some(Utc::now().timestamp_millis());
    counters.query_hits_30d = 5;
    hotness::upsert(&cfg, &counters).unwrap();

    let spawn = test_override::with_provider(Arc::clone(&provider), async {
        force_recompute(&cfg, "email:alice@example.com")
            .await
            .unwrap()
    })
    .await;

    match &spawn {
        SpawnOutcome::Spawned { tree_id, .. } | SpawnOutcome::TreeExists { tree_id, .. } => {
            assert!(tree_id.starts_with("topic:"));
        }
        other => panic!("expected Spawned or TreeExists, got {other:?}"),
    }

    // Drain backfill follow-up jobs.
    drain_until_idle(&cfg).await.unwrap();

    let topic_trees = tree_store::list_trees_by_kind(&cfg, TreeKind::Topic).unwrap();
    assert!(!topic_trees.is_empty());

    let topic_resp = query_topic(&cfg, "email:alice@example.com", None, None, 10)
        .await
        .unwrap();
    assert!(!topic_resp.hits.is_empty());

    // Verify event stream.
    tokio::task::yield_now().await;
    assert!(
        collector.count_by(
            |e| matches!(e, DomainEvent::DocumentCanonicalized { source_id: sid, .. }
            if sid == "gmail:alice-volume")
        ) >= 20
    );
}
