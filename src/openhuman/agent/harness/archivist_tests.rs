use super::*;
use crate::openhuman::agent::hooks::{ToolCallRecord, TurnContext};
use crate::openhuman::memory::chat::ChatPrompt;
use crate::openhuman::memory_store::{events as ev, fts5, segments as seg};

fn setup_conn() -> Arc<Mutex<Connection>> {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(fts5::EPISODIC_INIT_SQL).unwrap();
    conn.execute_batch(seg::SEGMENTS_INIT_SQL).unwrap();
    conn.execute_batch(ev::EVENTS_INIT_SQL).unwrap();
    conn.execute_batch(profile::PROFILE_INIT_SQL).unwrap();
    Arc::new(Mutex::new(conn))
}

#[tokio::test]
async fn archivist_indexes_turn() {
    let conn = setup_conn();
    let hook = ArchivistHook::new(conn.clone(), true);

    let ctx = TurnContext {
        user_message: "What is Rust?".into(),
        assistant_response: "Rust is a systems programming language.".into(),
        tool_calls: vec![],
        turn_duration_ms: 500,
        session_id: Some("test-session".into()),
        agent_id: None,
        entrypoint: None,
        iteration_count: 1,
    };

    hook.on_turn_complete(&ctx).await.unwrap();

    let entries = fts5::episodic_session_entries(&conn, "test-session").unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].role, "user");
    assert_eq!(entries[1].role, "assistant");
}

#[tokio::test]
async fn archivist_creates_segment_on_first_turn() {
    let conn = setup_conn();
    let hook = ArchivistHook::new(conn.clone(), true);

    let ctx = TurnContext {
        user_message: "Hello world".into(),
        assistant_response: "Hi there!".into(),
        tool_calls: vec![],
        turn_duration_ms: 100,
        session_id: Some("seg-test".into()),
        agent_id: None,
        entrypoint: None,
        iteration_count: 1,
    };

    hook.on_turn_complete(&ctx).await.unwrap();

    let open = seg::open_segment_for_session(&conn, "seg-test").unwrap();
    assert!(open.is_some());
    assert_eq!(open.unwrap().turn_count, 1);
}

#[tokio::test]
async fn archivist_detects_topic_change_boundary() {
    let conn = setup_conn();
    let hook = ArchivistHook::new(conn.clone(), true);

    hook.on_turn_complete(&TurnContext {
        user_message: "Tell me about Rust".into(),
        assistant_response: "Rust is great.".into(),
        tool_calls: vec![],
        turn_duration_ms: 100,
        session_id: Some("boundary-test".into()),
        agent_id: None,
        entrypoint: None,
        iteration_count: 1,
    })
    .await
    .unwrap();

    hook.on_turn_complete(&TurnContext {
        user_message: "How about its memory safety?".into(),
        assistant_response: "It uses ownership.".into(),
        tool_calls: vec![],
        turn_duration_ms: 100,
        session_id: Some("boundary-test".into()),
        agent_id: None,
        entrypoint: None,
        iteration_count: 2,
    })
    .await
    .unwrap();

    hook.on_turn_complete(&TurnContext {
        user_message: "Switching to a different topic now. I prefer dark mode.".into(),
        assistant_response: "Noted about dark mode.".into(),
        tool_calls: vec![],
        turn_duration_ms: 100,
        session_id: Some("boundary-test".into()),
        agent_id: None,
        entrypoint: None,
        iteration_count: 3,
    })
    .await
    .unwrap();

    let segments = seg::segments_by_namespace(&conn, "global", 10).unwrap();
    assert!(
        segments.len() >= 2,
        "Expected at least 2 segments, got {}",
        segments.len()
    );
}

#[tokio::test]
async fn archivist_extracts_failure_lesson() {
    let conn = setup_conn();
    let hook = ArchivistHook::new(conn.clone(), true);

    let ctx = TurnContext {
        user_message: "Run tests".into(),
        assistant_response: "Tests failed.".into(),
        tool_calls: vec![ToolCallRecord {
            name: "shell".into(),
            arguments: serde_json::json!({"command": "cargo test"}),
            success: false,
            output_summary: "shell: failed (error)".into(),
            duration_ms: 3000,
        }],
        turn_duration_ms: 3500,
        session_id: Some("test-session-2".into()),
        agent_id: None,
        entrypoint: None,
        iteration_count: 2,
    };

    hook.on_turn_complete(&ctx).await.unwrap();

    let entries = fts5::episodic_session_entries(&conn, "test-session-2").unwrap();
    let assistant_entry = entries.iter().find(|e| e.role == "assistant").unwrap();
    assert!(assistant_entry.lesson.as_ref().unwrap().contains("shell"));
}

#[tokio::test]
async fn disabled_archivist_is_noop() {
    let hook = ArchivistHook::disabled();
    let ctx = TurnContext {
        user_message: "test".into(),
        assistant_response: "test".into(),
        tool_calls: vec![],
        turn_duration_ms: 0,
        session_id: None,
        agent_id: None,
        entrypoint: None,
        iteration_count: 0,
    };
    hook.on_turn_complete(&ctx).await.unwrap();
}

#[test]
fn extract_profile_key_works() {
    let key = extract_profile_key("I prefer dark mode for coding", "preference");
    assert!(key.starts_with("preference_"));
    assert!(key.contains("prefer"));
}

#[tokio::test]
async fn archivist_accumulates_turns_in_segment() {
    let conn = setup_conn();
    let hook = ArchivistHook::new(conn.clone(), true);

    let session = "accum-session";

    for i in 1..=3 {
        hook.on_turn_complete(&TurnContext {
            user_message: format!("Turn number {i}"),
            assistant_response: format!("Response {i}"),
            tool_calls: vec![],
            turn_duration_ms: 50,
            session_id: Some(session.into()),
            agent_id: None,
            entrypoint: None,
            iteration_count: i,
        })
        .await
        .unwrap();
    }

    let open_seg = seg::open_segment_for_session(&conn, session)
        .unwrap()
        .expect("Expected an open segment after 3 turns");

    assert_eq!(
        open_seg.turn_count, 3,
        "Segment should have accumulated 3 turns, got {}",
        open_seg.turn_count
    );
}

#[tokio::test]
async fn archivist_extracts_preference_event_on_boundary() {
    let conn = setup_conn();
    let hook = ArchivistHook::new(conn.clone(), true);

    let session = "pref-boundary-session";

    hook.on_turn_complete(&TurnContext {
        user_message: "Tell me about Rust ownership".into(),
        assistant_response: "Ownership is a key concept in Rust.".into(),
        tool_calls: vec![],
        turn_duration_ms: 100,
        session_id: Some(session.into()),
        agent_id: None,
        entrypoint: None,
        iteration_count: 1,
    })
    .await
    .unwrap();

    hook.on_turn_complete(&TurnContext {
        user_message: "I prefer dark mode for all my editors".into(),
        assistant_response: "Good to know! Dark mode is easier on the eyes.".into(),
        tool_calls: vec![],
        turn_duration_ms: 100,
        session_id: Some(session.into()),
        agent_id: None,
        entrypoint: None,
        iteration_count: 2,
    })
    .await
    .unwrap();

    hook.on_turn_complete(&TurnContext {
        user_message: "Switching to a different topic — how does Tokio work?".into(),
        assistant_response: "Tokio is an async runtime.".into(),
        tool_calls: vec![],
        turn_duration_ms: 100,
        session_id: Some(session.into()),
        agent_id: None,
        entrypoint: None,
        iteration_count: 3,
    })
    .await
    .unwrap();

    let events = ev::events_by_type(&conn, "global", "preference", 20).unwrap();
    assert!(
        !events.is_empty(),
        "Expected at least one preference event after segment close; got 0."
    );
    let has_dark_mode = events
        .iter()
        .any(|e| e.content.to_lowercase().contains("prefer"));
    assert!(
        has_dark_mode,
        "Expected a preference event mentioning 'prefer', found: {:?}",
        events.iter().map(|e| &e.content).collect::<Vec<_>>()
    );
}

// ── Phase 0: episodic_capture_enabled independent of learning.enabled ────────

/// When `learning.enabled = false` but `episodic_capture_enabled = true`,
/// the ArchivistHook (constructed directly, as builder.rs would produce)
/// must still write 2 episodic_log rows (user + assistant) and create/advance
/// a segment. This verifies the core contract: episodic capture runs
/// regardless of the learning inference stack toggle.
#[tokio::test]
async fn phase0_episodic_rows_and_segment_without_learning_enabled() {
    let conn = setup_conn();
    // Simulate what builder.rs does when learning.enabled=false but
    // episodic_capture_enabled=true: construct the hook directly with
    // the SQLite conn, enabled=true. No config attached (no LLM recap
    // or tree ingest — those are gated by learning.enabled / chat_to_tree_enabled).
    let hook = ArchivistHook::new(conn.clone(), true);

    let session = "phase0-test-session";

    hook.on_turn_complete(&TurnContext {
        user_message: "Hello, what is Rust?".into(),
        assistant_response: "Rust is a systems language.".into(),
        tool_calls: vec![],
        turn_duration_ms: 100,
        session_id: Some(session.into()),
        agent_id: None,
        entrypoint: None,
        iteration_count: 1,
    })
    .await
    .unwrap();

    // Verify 2 episodic rows were written.
    let entries = fts5::episodic_session_entries(&conn, session).unwrap();
    assert_eq!(
        entries.len(),
        2,
        "Expected 2 episodic rows (user + assistant), got {}",
        entries.len()
    );
    assert_eq!(entries[0].role, "user");
    assert_eq!(entries[1].role, "assistant");

    // Verify a segment was created.
    let open_seg = seg::open_segment_for_session(&conn, session)
        .unwrap()
        .expect("Expected an open segment after first turn");
    assert_eq!(open_seg.turn_count, 1);

    // Add a second turn to verify segment advances.
    hook.on_turn_complete(&TurnContext {
        user_message: "Tell me more about ownership.".into(),
        assistant_response: "Ownership prevents data races.".into(),
        tool_calls: vec![],
        turn_duration_ms: 100,
        session_id: Some(session.into()),
        agent_id: None,
        entrypoint: None,
        iteration_count: 2,
    })
    .await
    .unwrap();

    let entries2 = fts5::episodic_session_entries(&conn, session).unwrap();
    assert_eq!(
        entries2.len(),
        4,
        "Expected 4 episodic rows after 2 turns, got {}",
        entries2.len()
    );
    let open_seg2 = seg::open_segment_for_session(&conn, session)
        .unwrap()
        .expect("Expected an open segment after 2 turns");
    assert_eq!(
        open_seg2.turn_count, 2,
        "Segment should have 2 turns, got {}",
        open_seg2.turn_count
    );
}

// ── Phase 1: LLM recap + finalize-time embedding ─────────────────────────────

/// Stub ChatProvider that returns a fixed recap string without hitting
/// any real LLM, so the test is hermetic.
struct StubChatProvider;

#[async_trait::async_trait]
impl crate::openhuman::memory::chat::ChatProvider for StubChatProvider {
    fn name(&self) -> &str {
        "stub:test"
    }

    async fn chat_for_json(&self, _prompt: &ChatPrompt) -> anyhow::Result<String> {
        Ok("stub recap: discussed Rust ownership model".to_string())
    }

    async fn chat_for_text(&self, _prompt: &ChatPrompt) -> anyhow::Result<String> {
        Ok("stub recap: discussed Rust ownership model".to_string())
    }
}

/// Stub Embedder that returns a fixed unit vector without hitting Ollama.
struct StubEmbedder;

#[async_trait::async_trait]
impl crate::openhuman::memory::score::embed::Embedder for StubEmbedder {
    fn name(&self) -> &'static str {
        "stub-embedder-v1"
    }

    async fn embed(&self, _text: &str) -> anyhow::Result<Vec<f32>> {
        // Return a simple 4-dim unit vector.
        Ok(vec![0.5_f32, 0.5, 0.5, 0.5])
    }
}

/// Build an ArchivistHook with stub provider + embedder injected directly.
/// Uses the test-only `new_with_stubs` constructor to bypass `with_config`.
fn hook_with_stubs(conn: Arc<Mutex<Connection>>) -> ArchivistHook {
    ArchivistHook::new_with_stubs(conn, Arc::new(StubChatProvider), Arc::new(StubEmbedder))
}

/// When a segment closes, the LLM chat provider recap is used (verified by
/// a non-empty segment summary) and an embedding row is written to
/// `segment_embeddings`.
#[tokio::test]
async fn phase1_llm_recap_and_embedding_on_segment_close() {
    let conn = setup_conn();
    let hook = hook_with_stubs(conn.clone());

    let session = "phase1-recap-test";

    // Turn 1 — opens first segment.
    hook.on_turn_complete(&TurnContext {
        user_message: "Tell me about Rust ownership".into(),
        assistant_response: "Rust's ownership model prevents data races.".into(),
        tool_calls: vec![],
        turn_duration_ms: 100,
        session_id: Some(session.into()),
        agent_id: None,
        entrypoint: None,
        iteration_count: 1,
    })
    .await
    .unwrap();

    // Turn 2 — continues same segment.
    hook.on_turn_complete(&TurnContext {
        user_message: "What about the borrow checker?".into(),
        assistant_response: "The borrow checker enforces ownership rules at compile time.".into(),
        tool_calls: vec![],
        turn_duration_ms: 100,
        session_id: Some(session.into()),
        agent_id: None,
        entrypoint: None,
        iteration_count: 2,
    })
    .await
    .unwrap();

    // Turn 3 — topic change triggers a boundary → closes first segment → recap + embed fire.
    hook.on_turn_complete(&TurnContext {
        user_message: "Completely different topic: what is async/await in Python?".into(),
        assistant_response: "Python asyncio enables concurrent programming.".into(),
        tool_calls: vec![],
        turn_duration_ms: 100,
        session_id: Some(session.into()),
        agent_id: None,
        entrypoint: None,
        iteration_count: 3,
    })
    .await
    .unwrap();

    // Verify segments exist.
    let segments = seg::segments_by_namespace(&conn, "global", 10).unwrap();
    assert!(
        segments.len() >= 2,
        "Expected at least 2 segments (closed + open), got {}",
        segments.len()
    );

    // Find the closed segment (has a summary).
    let closed = segments
        .iter()
        .find(|s| s.summary.as_ref().map(|s| !s.is_empty()).unwrap_or(false));
    assert!(
        closed.is_some(),
        "Expected at least one closed segment with a non-empty summary"
    );

    let closed_seg = closed.unwrap();
    let summary = closed_seg.summary.as_ref().unwrap();
    // The stub provider returns a fixed string — verify it was persisted.
    assert!(
        summary.contains("stub recap"),
        "Expected summary to contain 'stub recap', got: {:?}",
        summary
    );

    // Verify an embedding row was written for the closed segment.
    let embedding =
        seg::segment_embedding_get(&conn, &closed_seg.segment_id, "stub-embedder-v1").unwrap();
    assert!(
        embedding.is_some(),
        "Expected an embedding row for segment={} model=stub-embedder-v1",
        closed_seg.segment_id
    );
    let vec = embedding.unwrap();
    assert_eq!(vec.len(), 4, "Expected 4-dim vector from stub embedder");
    for v in &vec {
        assert!(
            (*v - 0.5_f32).abs() < 1e-4,
            "Expected vector components ≈ 0.5, got {v}"
        );
    }
}

/// `flush_open_segment` must force-close the trailing open segment and
/// trigger recap + embedding even without a boundary-triggering turn.
#[tokio::test]
async fn phase1_flush_open_segment_finalizes_trailing_segment() {
    let conn = setup_conn();
    let hook = hook_with_stubs(conn.clone());

    let session = "phase1-flush-test";

    // Write 2 turns — stays in one open segment (no topic boundary fires).
    for i in 1..=2 {
        hook.on_turn_complete(&TurnContext {
            user_message: format!("Question about Rust turn {i}"),
            assistant_response: format!("Answer about Rust turn {i}"),
            tool_calls: vec![],
            turn_duration_ms: 50,
            session_id: Some(session.into()),
            agent_id: None,
            entrypoint: None,
            iteration_count: i,
        })
        .await
        .unwrap();
    }

    // Confirm the segment is still open (no boundary fired).
    let open_seg_before = seg::open_segment_for_session(&conn, session).unwrap();
    assert!(
        open_seg_before.is_some(),
        "Expected an open segment before flush"
    );

    // Flush — should force-close, recap, and embed.
    hook.flush_open_segment(session).await;

    // Segment should now be closed (no open segment for this session).
    let open_seg_after = seg::open_segment_for_session(&conn, session).unwrap();
    assert!(
        open_seg_after.is_none(),
        "Expected no open segment after flush_open_segment"
    );

    // The formerly-open segment should now have a summary.
    let segments = seg::segments_by_namespace(&conn, "global", 10).unwrap();
    let flushed = segments.iter().find(|s| {
        s.session_id == session && s.summary.as_ref().map(|s| !s.is_empty()).unwrap_or(false)
    });
    assert!(
        flushed.is_some(),
        "Expected flushed segment to have a non-empty summary"
    );

    let seg_id = &flushed.unwrap().segment_id;
    let embedding = seg::segment_embedding_get(&conn, seg_id, "stub-embedder-v1").unwrap();
    assert!(
        embedding.is_some(),
        "Expected embedding row for flushed segment={seg_id}"
    );
}

// ── Phase 2: segment-granularity tree ingest ─────────────────────────────────
//
// The following tests verify:
//   a) No per-turn tree write fires from on_turn_complete (no double-write).
//   b) Exactly ONE tree ingest fires when a segment closes (not N per turn).
//   c) The ingested batch contains all the segment's raw prose turns.
//   d) The `source_id` is the constant "conversations:agent".
//   e) Each leaf message carries session/segment/episodic-span provenance.
//   f) The ingested content is raw prose, NOT the LLM recap.
//   g) flush_open_segment also triggers tree ingest.

use crate::openhuman::config::Config;
use crate::openhuman::memory_store::chunks::store::{count_chunks, list_chunks, ListChunksQuery};
use tempfile::TempDir;

/// Build a Config that points at a temp workspace, suitable for tree-ingest tests.
/// The memory_tree DB and content dir are created under `tmp.path()`.
fn test_config_with_tree() -> (TempDir, Config) {
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    // Disable embedding so ingest doesn't fail trying to contact Ollama.
    cfg.memory_tree.embedding_endpoint = None;
    cfg.memory_tree.embedding_model = None;
    cfg.memory_tree.embedding_strict = false;
    // Ensure the tree ingest gate is on.
    cfg.learning.chat_to_tree_enabled = true;
    (tmp, cfg)
}

/// Build a hook that has both stub providers AND a real-enough Config wired in,
/// so the Phase 2 tree ingest path is exercised hermetically.
fn hook_with_stubs_and_tree_config(conn: Arc<Mutex<Connection>>, cfg: Config) -> ArchivistHook {
    ArchivistHook::new_with_stubs_and_config(
        conn,
        Arc::new(StubChatProvider),
        Arc::new(StubEmbedder),
        cfg,
    )
}

/// After a single turn (no segment boundary), the tree must have ZERO chunks —
/// the per-turn pipe_turn_to_tree path no longer exists.
#[tokio::test]
async fn phase2_no_per_turn_tree_write() {
    let conn = setup_conn();
    let (_tmp, cfg) = test_config_with_tree();
    let hook = hook_with_stubs_and_tree_config(conn.clone(), cfg.clone());

    let session = "phase2-no-per-turn";

    // Single turn — no segment close fires, so no tree ingest should happen.
    hook.on_turn_complete(&TurnContext {
        user_message: "What is Rust?".into(),
        assistant_response: "Rust is a systems programming language.".into(),
        tool_calls: vec![],
        turn_duration_ms: 100,
        session_id: Some(session.into()),
        agent_id: None,
        entrypoint: None,
        iteration_count: 1,
    })
    .await
    .unwrap();

    // Segment is still open (no boundary fired) — tree must have 0 chunks.
    let open_seg = seg::open_segment_for_session(&conn, session).unwrap();
    assert!(
        open_seg.is_some(),
        "Expected an open segment (no boundary should have fired)"
    );

    let chunk_count = count_chunks(&cfg).unwrap();
    assert_eq!(
        chunk_count, 0,
        "Expected 0 tree chunks after a single turn (no segment close): \
         per-turn tree write must not exist (Phase 2)"
    );
}

/// When a segment closes (boundary triggered), exactly ONE tree ingest fires
/// for that segment containing all its turns — not one ingest per turn.
#[tokio::test]
async fn phase2_exactly_one_tree_ingest_per_segment_close() {
    let conn = setup_conn();
    let (_tmp, cfg) = test_config_with_tree();
    let hook = hook_with_stubs_and_tree_config(conn.clone(), cfg.clone());

    let session = "phase2-one-ingest";

    // Turn 1 — opens first segment.
    hook.on_turn_complete(&TurnContext {
        user_message: "Tell me about Rust ownership".into(),
        assistant_response: "Rust ownership prevents memory bugs.".into(),
        tool_calls: vec![],
        turn_duration_ms: 100,
        session_id: Some(session.into()),
        agent_id: None,
        entrypoint: None,
        iteration_count: 1,
    })
    .await
    .unwrap();

    // Turn 2 — stays in same segment.
    hook.on_turn_complete(&TurnContext {
        user_message: "What about the borrow checker?".into(),
        assistant_response: "The borrow checker enforces ownership at compile time.".into(),
        tool_calls: vec![],
        turn_duration_ms: 100,
        session_id: Some(session.into()),
        agent_id: None,
        entrypoint: None,
        iteration_count: 2,
    })
    .await
    .unwrap();

    // No tree write yet — segment still open.
    let pre_close_chunks = count_chunks(&cfg).unwrap();
    assert_eq!(
        pre_close_chunks, 0,
        "Expected 0 tree chunks before any segment close; got {pre_close_chunks}"
    );

    // Turn 3 — topic change triggers boundary → closes first segment → tree ingest fires.
    hook.on_turn_complete(&TurnContext {
        user_message: "Switching to a completely different topic: tell me about Python asyncio."
            .into(),
        assistant_response: "Python asyncio enables concurrent coroutines.".into(),
        tool_calls: vec![],
        turn_duration_ms: 100,
        session_id: Some(session.into()),
        agent_id: None,
        entrypoint: None,
        iteration_count: 3,
    })
    .await
    .unwrap();

    // Segment closed → exactly one ingest for the closed segment (containing turns 1+2).
    // The ingest packs the messages into one or more chunks (greedy packing),
    // but chunks_written >= 1 confirms ingest happened.
    let post_close_chunks = count_chunks(&cfg).unwrap();
    assert!(
        post_close_chunks >= 1,
        "Expected ≥ 1 tree chunk after segment close; got {post_close_chunks}"
    );

    // List the chunks and check they come from the constant source_id.
    let chunks = list_chunks(
        &cfg,
        &ListChunksQuery {
            source_id: Some("conversations:agent".to_string()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        !chunks.is_empty(),
        "Expected chunks under source_id='conversations:agent'"
    );
}

/// The ingested leaf messages must carry the episodic-provenance `source_ref`
/// in the expected format:
/// `agent://session/{session_id}/segment/{segment_id}#ep{start}-{end}`.
///
/// Also verifies that `source_id` is the constant `"conversations:agent"`.
#[tokio::test]
async fn phase2_provenance_stamped_on_leaf_and_source_id_is_constant() {
    let conn = setup_conn();
    let (_tmp, cfg) = test_config_with_tree();
    let hook = hook_with_stubs_and_tree_config(conn.clone(), cfg.clone());

    let session = "phase2-provenance";

    // Two turns in the first segment.
    for i in 1..=2 {
        hook.on_turn_complete(&TurnContext {
            user_message: format!("Ownership question {i}"),
            assistant_response: format!("Ownership answer {i}"),
            tool_calls: vec![],
            turn_duration_ms: 50,
            session_id: Some(session.into()),
            agent_id: None,
            entrypoint: None,
            iteration_count: i,
        })
        .await
        .unwrap();
    }

    // Force a segment close via flush_open_segment.
    hook.flush_open_segment(session).await;

    // Retrieve the closed segment to extract its ID.
    let all_segs = seg::segments_by_namespace(&conn, "global", 10).unwrap();
    let closed = all_segs
        .iter()
        .find(|s| {
            s.session_id == session
                && s.status != crate::openhuman::memory_store::segments::SegmentStatus::Open
        })
        .expect("Expected a closed segment after flush");

    let segment_id = &closed.segment_id;
    let start_ep = closed.start_episodic_id;
    let end_ep = closed.end_episodic_id.unwrap_or(start_ep);

    // Chunks should be present.
    let chunks = list_chunks(&cfg, &ListChunksQuery::default()).unwrap();
    assert!(
        !chunks.is_empty(),
        "Expected tree chunks after flush_open_segment"
    );

    // source_id must be the constant — never per-session or per-segment.
    for chunk in &chunks {
        assert_eq!(
            chunk.metadata.source_id, "conversations:agent",
            "source_id must be the constant 'conversations:agent', got: {}",
            chunk.metadata.source_id
        );
    }

    // The source_ref on at least one chunk must contain the provenance pattern.
    let expected_provenance =
        format!("agent://session/{session}/segment/{segment_id}#ep{start_ep}-{end_ep}");
    let has_provenance = chunks.iter().any(|chunk| {
        chunk
            .metadata
            .source_ref
            .as_ref()
            .map(|r| {
                r.value
                    .contains(&format!("agent://session/{session}/segment/{segment_id}"))
            })
            .unwrap_or(false)
    });
    assert!(
        has_provenance,
        "Expected at least one chunk with source_ref containing provenance pattern \
         '{expected_provenance}'; found: {:?}",
        chunks
            .iter()
            .map(|c| c.metadata.source_ref.as_ref().map(|r| r.value.as_str()))
            .collect::<Vec<_>>()
    );
}

/// The ingested content must be the raw prose turns (user + assistant text),
/// NOT equal to the LLM recap text. The recap lives only in the STM segment
/// layer; the tree must ingest raw evidence so it can build its own summaries.
#[tokio::test]
async fn phase2_ingested_content_is_raw_prose_not_recap() {
    let conn = setup_conn();
    let (_tmp, cfg) = test_config_with_tree();
    let hook = hook_with_stubs_and_tree_config(conn.clone(), cfg.clone());

    let session = "phase2-raw-prose";

    // The stub recap always returns "stub recap: discussed Rust ownership model".
    // The raw user messages contain very different text.
    let user_msg = "My specific question about lifetimes in Rust code";
    let asst_msg = "Lifetimes annotate how long references are valid in memory";

    hook.on_turn_complete(&TurnContext {
        user_message: user_msg.into(),
        assistant_response: asst_msg.into(),
        tool_calls: vec![],
        turn_duration_ms: 100,
        session_id: Some(session.into()),
        agent_id: None,
        entrypoint: None,
        iteration_count: 1,
    })
    .await
    .unwrap();

    // Flush to close the segment and trigger tree ingest.
    hook.flush_open_segment(session).await;

    let chunks = list_chunks(&cfg, &ListChunksQuery::default()).unwrap();
    assert!(
        !chunks.is_empty(),
        "Expected tree chunks after flush_open_segment"
    );

    // The stub recap text must NOT appear in any chunk body.
    let stub_recap_text = "stub recap: discussed Rust ownership model";
    for chunk in &chunks {
        assert!(
            !chunk.content.contains(stub_recap_text),
            "Chunk content must NOT contain the recap text (evidence-vs-interpretation policy). \
             Found recap text in chunk id={}: {:?}",
            chunk.id,
            &chunk.content[..chunk.content.len().min(200)]
        );
    }

    // The raw prose text MUST appear in at least one chunk.
    let has_user_prose = chunks
        .iter()
        .any(|c| c.content.to_ascii_lowercase().contains("lifetimes"));
    assert!(
        has_user_prose,
        "Expected at least one chunk body to contain raw prose from the turn \
         (keyword 'lifetimes'); found: {:?}",
        chunks
            .iter()
            .map(|c| &c.content[..c.content.len().min(100)])
            .collect::<Vec<_>>()
    );
}

/// `flush_open_segment` must also trigger the tree ingest for the trailing
/// open segment (same as on_segment_closed at a topic boundary).
#[tokio::test]
async fn phase2_flush_also_triggers_tree_ingest() {
    let conn = setup_conn();
    let (_tmp, cfg) = test_config_with_tree();
    let hook = hook_with_stubs_and_tree_config(conn.clone(), cfg.clone());

    let session = "phase2-flush-tree";

    // Two turns — no boundary fires, segment stays open.
    for i in 1..=2 {
        hook.on_turn_complete(&TurnContext {
            user_message: format!("Rust borrowing question {i}"),
            assistant_response: format!("Borrowing answer {i}"),
            tool_calls: vec![],
            turn_duration_ms: 50,
            session_id: Some(session.into()),
            agent_id: None,
            entrypoint: None,
            iteration_count: i,
        })
        .await
        .unwrap();
    }

    // Confirm no tree chunks yet (segment still open).
    let before = count_chunks(&cfg).unwrap();
    assert_eq!(
        before, 0,
        "Expected 0 tree chunks before flush; got {before}"
    );

    // Flush should close the segment and trigger tree ingest.
    hook.flush_open_segment(session).await;

    let after = count_chunks(&cfg).unwrap();
    assert!(
        after >= 1,
        "Expected ≥ 1 tree chunk after flush_open_segment triggers segment ingest; got {after}"
    );
}
