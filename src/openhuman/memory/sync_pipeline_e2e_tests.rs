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

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::openhuman::config::Config;
use tinybus::EventHandler;
use tinybus::SubscriptionHandle;
// Named on the engine crate directly: `memory::tree::retrieval` stopped
// re-exporting the engine in #5560 because no production caller was left. A
// test may still reach the engine — that is what keeps this a test-only
// reference rather than a shipped one.
use tinycortex::memory::ingest::canonicalize::chat::{ChatBatch, ChatMessage};
use tinymemory_api::sync_events::{emit_sync_stage, MemorySyncStage, MemorySyncTrigger};
use tinymemory_core::ingest_pipeline::ingest_chat;
use tinymemory_core::queue::{self as memory_queue, count_total, drain_until_idle, JobStatus};
use tinymemory_core::store::chunks::store::{
    count_chunks, count_chunks_by_lifecycle_status, CHUNK_STATUS_BUFFERED,
};
use tinymemory_core::store::trees::{store as tree_store, types::TreeKind};
use tinymemory_core::tree::retrieval::{query_source, search_entities};
use tinymemory_core::tree::score::store::lookup_entity;

// ── helpers ─────────────────────────────────────────────────────────────

fn test_config() -> (TempDir, Config) {
    // Ingestion canonicalises through the host seams, so they must be wired.
    // This module never installed them and passed only when some other test in
    // the binary had; filtered to this file it failed outright. `Once`-guarded,
    // so this is free when another test got there first.
    crate::openhuman::memory::host_impls::install_for_tests();
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    cfg.memory_tree.embedding_endpoint = None;
    cfg.memory_tree.embedding_model = None;
    cfg.memory_tree.embedding_strict = false;
    cfg.embeddings_provider = Some("none".to_string());
    (tmp, cfg)
}

fn failed_job_diagnostics(cfg: &Config) -> Vec<(String, i64, Option<String>)> {
    tinymemory_core::store::chunks::with_connection(cfg, |conn| {
        let mut stmt = conn.prepare(
            "SELECT kind, attempts, last_error FROM mem_tree_jobs \
             WHERE status = 'failed' ORDER BY created_at_ms",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    })
    .unwrap()
}

async fn ensure_event_bus() {
    // Standing the bus up is async now — it connects to a broker. Idempotent.
    crate::core::bus::init().await.expect("bus init");
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
        let handle = BUS.subscribe(Arc::new(self.clone()));
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

    /// Wait until at least `want` events match, or fail on a deadline.
    ///
    /// A single `yield_now` used to be enough when the bus was a channel and
    /// the handler ran inline on the subscriber task. On tinybus an event
    /// crosses two task hops — the subscriber loop, then the isolated handler
    /// task — so a batch of twenty needs to be waited for rather than assumed.
    async fn wait_for<F: Fn(&DomainEvent) -> bool>(&self, want: usize, pred: F) -> usize {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let seen = self.count_by(&pred);
            if seen >= want || tokio::time::Instant::now() > deadline {
                return seen;
            }
            tokio::task::yield_now().await;
        }
    }
}

#[async_trait]
impl EventHandler<DomainEvent> for EventCollector {
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
    ensure_event_bus().await;

    let (collector, _handle) = EventCollector::new().subscribe();

    // Simulate sync lifecycle events.
    emit_sync_stage(
        MemorySyncTrigger::Manual,
        MemorySyncStage::Requested,
        Some("gmail"),
        Some("conn-1"),
        None,
        None, // channel-level — not a memory-source row
    );
    emit_sync_stage(
        MemorySyncTrigger::Manual,
        MemorySyncStage::Fetching,
        Some("gmail"),
        Some("conn-1"),
        None,
        None, // channel-level — not a memory-source row
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

    // DocumentCanonicalized event. Waited for, not assumed: the event crosses
    // two task hops on tinybus, so a bare `yield_now` raced the handler and
    // made this test flaky — it alternated pass/fail across identical runs.
    let canonicalized_count = collector
        .wait_for(1, |e| {
            matches!(e, DomainEvent::DocumentCanonicalized { source_kind, source_id: sid, .. }
                if source_kind == "chat" && sid == "gmail:alice-thread-1")
        })
        .await;
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
        None, // channel-level — not a memory-source row
    );

    // Same race as above. Waits for the **terminal** stage specifically, not
    // merely for some stage event: the assertions below require `completed` to
    // have arrived, and any earlier stage would satisfy a looser predicate
    // while the pipeline was still running.
    collector
        .wait_for(1, |e| {
            matches!(e, DomainEvent::MemorySyncStageChanged { stage, .. } if stage == "completed")
        })
        .await;
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
    ensure_event_bus().await;

    let (collector, _handle) = EventCollector::new().subscribe();

    let source_id = "gmail:alice-volume";
    let base_ts = Utc::now().timestamp_millis() - 86_400_000;

    // Ingest 30 batches with large bodies to cross the 50k token seal threshold.
    // Each large_body is ~302 tokens. 6 segments = ~1812 tokens per batch.
    // 30 batches * 1812 tokens = 54,360 tokens (> 50,000 threshold).
    // We vary each repetition slightly to ensure no content-based deduplication
    // collapses the volume.
    for i in 0..30u32 {
        let mut body = String::new();
        for j in 0..6 {
            body.push_str(&large_body(i));
            body.push_str(&format!("\n\nRepeat marker: batch {i} / segment {j}\n\n"));
        }
        let batch = mk_batch("gmail", "inbox", i, &body, base_ts);
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

    let canonicalized = collector
        .wait_for(20, |e| {
            matches!(e, DomainEvent::DocumentCanonicalized { source_id: sid, .. }
                if sid == "gmail:alice-volume")
        })
        .await;
    assert!(canonicalized >= 20, "got {canonicalized}");

    // A parallel test can briefly hold the process-global LLM gate, causing
    // the seal job to defer. Keep draining until that deferred work becomes
    // claimable and the durable tree state reflects the completed seal.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    let source_tree = loop {
        drain_until_idle(&cfg).await.unwrap();
        let failed_jobs = failed_job_diagnostics(&cfg);
        assert!(
            failed_jobs.is_empty(),
            "memory jobs failed before the source tree sealed: {failed_jobs:?}"
        );
        if let Some(tree) = tree_store::list_trees_by_kind(&cfg, TreeKind::Source)
            .unwrap()
            .into_iter()
            .find(|tree| tree.scope == source_id && tree.max_level >= 1)
        {
            break tree;
        }
        if tokio::time::Instant::now() >= deadline {
            let trees = tree_store::list_trees_by_kind(&cfg, TreeKind::Source).unwrap();
            let buffer = trees
                .iter()
                .find(|tree| tree.scope == source_id)
                .map(|tree| tree_store::get_buffer(&cfg, &tree.id, 0).unwrap());
            panic!(
                "source tree did not seal before timeout: trees={trees:?}, buffer={buffer:?}, \
                 ready={}, running={}, done={}, failed={}",
                memory_queue::count_by_status(&cfg, JobStatus::Ready).unwrap(),
                memory_queue::count_by_status(&cfg, JobStatus::Running).unwrap(),
                memory_queue::count_by_status(&cfg, JobStatus::Done).unwrap(),
                memory_queue::count_by_status(&cfg, JobStatus::Failed).unwrap(),
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };

    // Source tree should have sealed to L1+.
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

    // (The global-digest and topic-spawn steps were removed with those
    // trees — source trees plus the entity index are the substrate.)

    // Verify event stream. Twenty events across two task hops each — the
    // helper exists precisely because a single yield cannot cover that.
    let canonicalized = collector
        .wait_for(20, |e| {
            matches!(e, DomainEvent::DocumentCanonicalized { source_id: sid, .. }
                if sid == "gmail:alice-volume")
        })
        .await;
    assert!(
        canonicalized >= 20,
        "expected 20 canonicalized events, saw {canonicalized}"
    );
}
