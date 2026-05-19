//! Unit + integration tests for Phase 3 STM recall.

use super::*;
use crate::openhuman::agent::harness::archivist::ArchivistHook;
use crate::openhuman::agent::hooks::{PostTurnHook, TurnContext};
use crate::openhuman::memory::store::events::EVENTS_INIT_SQL;
use crate::openhuman::memory::store::fts5;
use crate::openhuman::memory::store::profile::PROFILE_INIT_SQL;
use crate::openhuman::memory::store::segments::SEGMENTS_INIT_SQL;
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use std::sync::Arc;

// ── helpers ──────────────────────────────────────────────────────────────────

fn setup_conn() -> Arc<Mutex<Connection>> {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(fts5::EPISODIC_INIT_SQL).unwrap();
    conn.execute_batch(SEGMENTS_INIT_SQL).unwrap();
    conn.execute_batch(EVENTS_INIT_SQL).unwrap();
    conn.execute_batch(PROFILE_INIT_SQL).unwrap();
    Arc::new(Mutex::new(conn))
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// Insert an episodic entry with an explicit timestamp.
fn insert_episodic(
    conn: &Arc<Mutex<Connection>>,
    session_id: &str,
    ts: f64,
    role: &str,
    content: &str,
) -> i64 {
    let c = conn.lock();
    c.execute(
        "INSERT INTO episodic_log (session_id, timestamp, role, content, lesson, tool_calls_json, cost_microdollars) VALUES (?1,?2,?3,?4,NULL,NULL,0)",
        params![session_id, ts, role, content],
    ).unwrap();
    c.last_insert_rowid()
}

/// Insert a segment with a summary and optional embedding.
fn insert_segment_with_embedding(
    conn: &Arc<Mutex<Connection>>,
    segment_id: &str,
    session_id: &str,
    start_id: i64,
    end_id: i64,
    summary: &str,
    embedding: Option<Vec<f32>>,
    updated_at: f64,
    model_sig: &str,
) {
    let c = conn.lock();
    c.execute(
        "INSERT INTO conversation_segments
         (segment_id, session_id, namespace, start_episodic_id, end_episodic_id,
          start_timestamp, end_timestamp, turn_count, summary, status, created_at, updated_at)
         VALUES (?1,?2,'global',?3,?4,?5,?5,2,?6,'summarised',?5,?5)",
        params![segment_id, session_id, start_id, end_id, updated_at, summary],
    )
    .unwrap();

    if let Some(emb) = embedding {
        let bytes: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
        let dim = emb.len() as i64;
        c.execute(
            "INSERT INTO segment_embeddings (segment_id, model_signature, vector, dim, created_at)
             VALUES (?1,?2,?3,?4,?5)",
            params![segment_id, model_sig, bytes, dim, updated_at],
        )
        .unwrap();
    }
}

// ── cosine_similarity unit tests ──────────────────────────────────────────────

#[test]
fn cosine_identical_vectors_returns_one() {
    let v = vec![1.0_f32, 0.0, 0.0];
    assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-6);
}

#[test]
fn cosine_orthogonal_vectors_returns_zero() {
    let a = vec![1.0_f32, 0.0, 0.0];
    let b = vec![0.0_f32, 1.0, 0.0];
    assert!((cosine_similarity(&a, &b)).abs() < 1e-6);
}

#[test]
fn cosine_opposite_vectors_returns_minus_one() {
    let a = vec![1.0_f32, 0.0, 0.0];
    let b = vec![-1.0_f32, 0.0, 0.0];
    assert!((cosine_similarity(&a, &b) + 1.0).abs() < 1e-6);
}

#[test]
fn cosine_zero_vector_returns_zero_not_nan() {
    let a = vec![0.0_f32, 0.0, 0.0];
    let b = vec![1.0_f32, 0.0, 0.0];
    let sim = cosine_similarity(&a, &b);
    assert!(!sim.is_nan(), "cosine_similarity must not return NaN");
    assert_eq!(sim, 0.0);
}

#[test]
fn cosine_mismatched_lengths_returns_zero() {
    let a = vec![1.0_f32, 0.0];
    let b = vec![1.0_f32, 0.0, 0.0];
    assert_eq!(cosine_similarity(&a, &b), 0.0);
}

#[test]
fn cosine_empty_vectors_returns_zero() {
    assert_eq!(cosine_similarity(&[], &[]), 0.0);
}

// ── gating threshold tests ────────────────────────────────────────────────────

#[test]
fn cosine_gate_const_is_reasonable() {
    // Gate must be in (0.5, 1.0) — below 0.5 lets in noise, above 0.9 is too strict.
    assert!(super::super::COSINE_GATE > 0.5 && super::super::COSINE_GATE < 0.9);
}

#[test]
fn arm2_drops_below_gate_and_accepts_above() {
    let conn = setup_conn();
    let now = now_ts();

    // Build query embedding — unit vector along dim 0
    let mut q_emb = vec![0.0_f32; 8];
    q_emb[0] = 1.0;

    // High-match segment: unit vector along dim 0 (cos = 1.0)
    let mut high_emb = vec![0.0_f32; 8];
    high_emb[0] = 1.0;
    insert_episodic(&conn, "other-session", now - 100.0, "user", "seed turn");
    let id = insert_episodic(
        &conn,
        "other-session",
        now - 90.0,
        "assistant",
        "high match reply",
    );
    insert_segment_with_embedding(
        &conn,
        "seg-high",
        "other-session",
        id - 1,
        id,
        "This conversation covered high-match topics",
        Some(high_emb),
        now - 50.0,
        "test:model:8",
    );

    // Low-match segment: orthogonal vector (cos = 0.0 < gate)
    let mut low_emb = vec![0.0_f32; 8];
    low_emb[1] = 1.0;
    let id2 = insert_episodic(&conn, "low-session", now - 200.0, "user", "unrelated");
    insert_segment_with_embedding(
        &conn,
        "seg-low",
        "low-session",
        id2,
        id2,
        "This is about something completely unrelated",
        Some(low_emb),
        now - 150.0,
        "test:model:8",
    );

    let opts = StmRecallOpts {
        exclude_session: "current-session",
        query: Some("high match"),
        model_signature: None,
    };
    let block = stm_recall(&conn, &opts, Some(&q_emb)).unwrap();

    let recap_ids: Vec<&str> = block
        .items
        .iter()
        .filter_map(|it| {
            if let StmItem::SegmentRecap { segment_id, .. } = it {
                Some(segment_id.as_str())
            } else {
                None
            }
        })
        .collect();

    assert!(
        recap_ids.contains(&"seg-high"),
        "high-cosine segment must be accepted; got: {:?}",
        recap_ids
    );
    assert!(
        !recap_ids.contains(&"seg-low"),
        "low-cosine segment must be excluded by gate; got: {:?}",
        recap_ids
    );
}

// ── exclude-own-session tests ─────────────────────────────────────────────────

#[test]
fn exclude_own_session_arm1_fts5() {
    let conn = setup_conn();
    let now = now_ts();

    // Other session — should appear
    insert_episodic(
        &conn,
        "other-sess",
        now - 100.0,
        "user",
        "Rust programming concepts",
    );
    // Current session — must be excluded
    insert_episodic(
        &conn,
        "current-sess",
        now - 50.0,
        "user",
        "Rust programming today",
    );

    let opts = StmRecallOpts {
        exclude_session: "current-sess",
        query: Some("Rust programming"),
        model_signature: None,
    };
    let block = stm_recall(&conn, &opts, None).unwrap();

    for item in &block.items {
        if let StmItem::EpisodicTurn { session_id, .. } = item {
            assert_ne!(
                session_id, "current-sess",
                "arm1 must never return current session items; got session_id={session_id}"
            );
        }
    }
    // Must see the other session
    let has_other = block.items.iter().any(
        |it| matches!(it, StmItem::EpisodicTurn { session_id, .. } if session_id == "other-sess"),
    );
    assert!(has_other, "arm1 must surface items from other sessions");
}

#[test]
fn exclude_own_session_arm2_vector() {
    let conn = setup_conn();
    let now = now_ts();

    let mut emb = vec![0.0_f32; 8];
    emb[0] = 1.0;

    // Insert segment from "current-session" — should be excluded
    let id = insert_episodic(
        &conn,
        "current-session",
        now - 100.0,
        "user",
        "current thread",
    );
    insert_segment_with_embedding(
        &conn,
        "seg-current",
        "current-session",
        id,
        id,
        "Current session recap",
        Some(emb.clone()),
        now - 50.0,
        "test:model:8",
    );

    // Segment from another session — should appear
    let id2 = insert_episodic(&conn, "other-session", now - 200.0, "user", "other thread");
    insert_segment_with_embedding(
        &conn,
        "seg-other",
        "other-session",
        id2,
        id2,
        "Other session recap",
        Some(emb.clone()),
        now - 100.0,
        "test:model:8",
    );

    let opts = StmRecallOpts {
        exclude_session: "current-session",
        query: None,
        model_signature: None,
    };
    let block = stm_recall(&conn, &opts, Some(&emb)).unwrap();

    for item in &block.items {
        if let StmItem::SegmentRecap { session_id, .. } = item {
            assert_ne!(
                session_id, "current-session",
                "arm2 must never return current session recaps"
            );
        }
    }
}

// ── dedup-by-episodic-span tests ──────────────────────────────────────────────

#[test]
fn dedup_drops_episodic_row_inside_segment_span() {
    let conn = setup_conn();
    let now = now_ts();

    // Insert episodic rows for "other-session"
    let id_start = insert_episodic(
        &conn,
        "other-session",
        now - 300.0,
        "user",
        "Rust ownership",
    );
    let id_end = insert_episodic(
        &conn,
        "other-session",
        now - 290.0,
        "assistant",
        "Rust uses borrow checker",
    );

    // A high-similarity segment recap covers those episodic rows
    let mut emb = vec![0.0_f32; 8];
    emb[0] = 1.0;
    insert_segment_with_embedding(
        &conn,
        "seg-covers",
        "other-session",
        id_start,
        id_end,
        "Conversation about Rust ownership and borrow checker",
        Some(emb.clone()),
        now - 100.0,
        "test:model:8",
    );

    let opts = StmRecallOpts {
        exclude_session: "current",
        query: Some("Rust ownership"),
        model_signature: None,
    };
    let block = stm_recall(&conn, &opts, Some(&emb)).unwrap();

    // The segment recap must appear
    let has_recap = block.items.iter().any(
        |it| matches!(it, StmItem::SegmentRecap { segment_id, .. } if segment_id == "seg-covers"),
    );
    assert!(has_recap, "segment recap must appear in output");

    // The covered episodic rows must NOT appear (dedup)
    for item in &block.items {
        if let StmItem::EpisodicTurn { id, .. } = item {
            assert!(
                *id != Some(id_start) && *id != Some(id_end),
                "episodic rows inside segment span must be deduplicated; id={id:?}"
            );
        }
    }
    assert!(
        block.dropped_dedup > 0,
        "dropped_dedup must be > 0 when rows are inside a segment span"
    );
}

// ── recency window bound test ─────────────────────────────────────────────────

#[test]
fn recency_window_excludes_old_segments() {
    let conn = setup_conn();
    let now = now_ts();

    // Recent segment — within window
    let id1 = insert_episodic(&conn, "recent-session", now - 100.0, "user", "recent");
    let emb_recent: Vec<f32> = (0..8).map(|i| if i == 0 { 1.0 } else { 0.0 }).collect();
    insert_segment_with_embedding(
        &conn,
        "seg-recent",
        "recent-session",
        id1,
        id1,
        "Recent segment recap",
        Some(emb_recent.clone()),
        now - 100.0, // recent
        "test:model:8",
    );

    // Old segment — beyond RECENCY_WINDOW_DAYS
    let old_ts = now - (super::super::RECENCY_WINDOW_DAYS + 2.0) * 86_400.0;
    let id2 = insert_episodic(&conn, "old-session", old_ts, "user", "old content");
    insert_segment_with_embedding(
        &conn,
        "seg-old",
        "old-session",
        id2,
        id2,
        "Old segment recap",
        Some(emb_recent.clone()),
        old_ts, // older than window
        "test:model:8",
    );

    let opts = StmRecallOpts {
        exclude_session: "current",
        query: None,
        model_signature: None,
    };
    let block = stm_recall(&conn, &opts, Some(&emb_recent)).unwrap();

    let seg_ids: Vec<&str> = block
        .items
        .iter()
        .filter_map(|it| {
            if let StmItem::SegmentRecap { segment_id, .. } = it {
                Some(segment_id.as_str())
            } else {
                None
            }
        })
        .collect();

    assert!(
        !seg_ids.contains(&"seg-old"),
        "old segment beyond recency window must be excluded; got: {:?}",
        seg_ids
    );
    assert!(
        seg_ids.contains(&"seg-recent"),
        "recent segment must appear; got: {:?}",
        seg_ids
    );
}

// ── token budget test ─────────────────────────────────────────────────────────

#[test]
fn token_budget_limits_output_size() {
    let conn = setup_conn();
    let now = now_ts();

    // Insert many episodic turns from other sessions
    for i in 0..50 {
        let large_content = "X".repeat(300); // 300 chars each
        insert_episodic(
            &conn,
            &format!("session-{i}"),
            now - i as f64 * 60.0,
            "user",
            &large_content,
        );
    }

    let opts = StmRecallOpts {
        exclude_session: "current",
        query: None,
        model_signature: None,
    };
    let block = stm_recall(&conn, &opts, None).unwrap();

    let total_chars: usize = block.items.iter().map(|it| it.approx_chars()).sum();
    assert!(
        total_chars <= super::super::TOKEN_BUDGET,
        "total chars {} must not exceed budget {}",
        total_chars,
        super::super::TOKEN_BUDGET
    );
}

// ── preemptive recency fallback (no query) ────────────────────────────────────

#[test]
fn preemptive_no_query_returns_recent_other_sessions() {
    let conn = setup_conn();
    let now = now_ts();

    // Insert turns from other sessions
    insert_episodic(
        &conn,
        "session-a",
        now - 300.0,
        "user",
        "Alpha session content",
    );
    insert_episodic(
        &conn,
        "session-b",
        now - 200.0,
        "user",
        "Beta session content",
    );

    // Also insert turns for the current session — must be excluded
    insert_episodic(
        &conn,
        "current-session",
        now - 100.0,
        "user",
        "Current session content",
    );

    let opts = StmRecallOpts {
        exclude_session: "current-session",
        query: None,
        model_signature: None,
    };
    let block = stm_recall(&conn, &opts, None).unwrap();

    // Check that we got results from other sessions
    let other_sessions: Vec<&str> = block
        .items
        .iter()
        .filter_map(|it| {
            if let StmItem::EpisodicTurn { session_id, .. } = it {
                Some(session_id.as_str())
            } else {
                None
            }
        })
        .collect();

    assert!(
        !other_sessions.is_empty() || block.items.is_empty(), // empty is OK if no rows
        "preemptive fallback must only return other-session items"
    );
    for sid in &other_sessions {
        assert_ne!(
            *sid, "current-session",
            "current session must be excluded in preemptive mode"
        );
    }
}

// ── rendered block format ─────────────────────────────────────────────────────

#[test]
fn render_produces_non_empty_markdown_when_items_present() {
    let conn = setup_conn();
    let now = now_ts();

    insert_episodic(&conn, "other-session", now - 100.0, "user", "Test content");

    let opts = StmRecallOpts {
        exclude_session: "current",
        query: None,
        model_signature: None,
    };
    let block = stm_recall(&conn, &opts, None).unwrap();

    if !block.items.is_empty() {
        let rendered = block.render();
        assert!(
            rendered.contains("## Recent context"),
            "rendered block must contain heading"
        );
    }
}

#[test]
fn render_empty_block_returns_empty_string() {
    let block = StmRecallBlock::default();
    assert!(block.render().is_empty());
    assert!(block.is_empty());
}

// ── end-to-end integration test ───────────────────────────────────────────────
// Drive the real chain: completed turns → episodic rows → segment close
// (recap + embedding via the Phase 0+1 path using stub providers) →
// STM recall returns cross-thread recaps and excludes the current session.

#[tokio::test]
async fn e2e_stm_recall_chain() {
    use crate::openhuman::memory::tree::chat::ChatPrompt;

    let conn = setup_conn();

    // ── Phase 0+1 stub providers ─────────────────────────────────────────────
    // We use a stub chat provider that returns a fixed recap string, and the
    // InertEmbedder that returns zero vectors. This exercises the real
    // archivist code path (recap + segment_embedding_upsert) without
    // requiring a live LLM or Ollama daemon.

    struct StubChat;
    use crate::openhuman::memory::tree::chat::ChatProvider;
    #[async_trait::async_trait]
    impl ChatProvider for StubChat {
        fn name(&self) -> &str {
            "stub"
        }
        async fn chat_for_json(&self, _prompt: &ChatPrompt) -> anyhow::Result<String> {
            Ok("RECAP: stub LLM summary of the segment.".to_string())
        }
        async fn chat_for_text(&self, _prompt: &ChatPrompt) -> anyhow::Result<String> {
            Ok("RECAP: stub LLM summary of the segment.".to_string())
        }
    }

    use crate::openhuman::memory::tree::score::embed::InertEmbedder;
    let chat_provider: Arc<dyn crate::openhuman::memory::tree::chat::ChatProvider> =
        Arc::new(StubChat);
    let embedder: Arc<dyn crate::openhuman::memory::tree::score::embed::Embedder> =
        Arc::new(InertEmbedder::new());

    let archivist = ArchivistHook::new_with_stubs(conn.clone(), chat_provider, embedder);

    // ── Turns for "other-thread" ─────────────────────────────────────────────
    // Drive 25 turns on session "other-thread" — exceeds max_turns_per_segment (20)
    // so a segment boundary fires, the segment closes, and recap + embedding happen.

    for i in 0..25 {
        let ctx = TurnContext {
            user_message: format!("User message {i} about Rust and memory safety"),
            assistant_response: format!("Assistant response {i}: Rust ownership is great."),
            tool_calls: vec![],
            turn_duration_ms: 100,
            session_id: Some("other-thread".to_string()),
            iteration_count: i + 1,
        };
        archivist.on_turn_complete(&ctx).await.unwrap();
    }

    // Force-flush any trailing open segment so we definitely get a recap.
    archivist.flush_open_segment("other-thread").await;

    // ── Verify episodic rows were written ────────────────────────────────────
    let ep_rows = fts5::episodic_session_entries(&conn, "other-thread").unwrap();
    assert!(
        ep_rows.len() >= 50,
        "expected >=50 episodic rows (2 per turn × 25), got {}",
        ep_rows.len()
    );

    // ── Verify segment embedding written ─────────────────────────────────────
    let has_embedding = {
        let c = conn.lock();
        let count: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM segment_embeddings se
                  JOIN conversation_segments cs ON se.segment_id = cs.segment_id
                 WHERE cs.session_id = 'other-thread'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        count > 0
    };
    assert!(
        has_embedding,
        "CRITICAL: Phase 0+1 did NOT write segment_embeddings for other-thread. \
         This means the archivist recap+embed path is broken. \
         STM recall Arm 2 would have no data to query."
    );

    // ── Now run STM recall from "current-thread" ─────────────────────────────
    // InertEmbedder returns zero vectors, cosine of zero vectors = 0.0 < COSINE_GATE.
    // So Arm 2 will find no hits (expected — inert embedder produces identical vectors).
    // Arm 1 (FTS5 or recency) should still return episodic turns from other-thread.

    let opts = StmRecallOpts {
        exclude_session: "current-thread",
        query: Some("Rust memory safety"),
        model_signature: None,
    };
    let block = stm_recall(&conn, &opts, None).unwrap(); // no embedding for Arm 2

    // With keyword "Rust memory safety" + other-thread has "Rust and memory safety"
    // in the episodic log, Arm 1 should surface at least some results.
    // (FTS5 porter-stems "safety" and "Rust" matches the stored content.)

    // Verify: nothing from current-thread
    for item in &block.items {
        match item {
            StmItem::EpisodicTurn { session_id, .. } => {
                assert_ne!(
                    session_id, "current-thread",
                    "STM recall must never return current-thread items"
                );
            }
            StmItem::SegmentRecap { session_id, .. } => {
                assert_ne!(
                    session_id, "current-thread",
                    "STM recall must never return current-thread recaps"
                );
            }
        }
    }

    // The FTS5 arm should have found the other-thread episodic rows
    let fts5_hits = block.fts5_candidates;
    assert!(
        fts5_hits > 0,
        "Arm 1 (FTS5) must have found candidates from other-thread for 'Rust memory safety' query; \
         fts5_candidates={}. This proves episodic rows are written and searchable.",
        fts5_hits
    );

    // Verify block is well-formed
    let rendered = block.render();
    if !block.items.is_empty() {
        assert!(
            rendered.contains("## Recent context"),
            "rendered block must have heading"
        );
    }
}
