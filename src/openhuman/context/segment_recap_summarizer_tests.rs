//! Tests for Phase 1.5 — `SegmentRecapSummarizer`.
//!
//! Proves:
//!   1. Rolling recap summarizes the open segment WITHOUT closing it, writing
//!      `segment_set_summary`, or producing an embedding row.
//!   2. When a rolling recap is available, the compaction replacement text
//!      equals it (provenance/path), not a separately-generated summary.
//!   3. Soft-fallback: archivist absent / LLM stub failing / flag off →
//!      compaction falls back to the inner summarizer; prompt stays bounded.
//!   4. Finalize path (Phase 1 `on_segment_closed`) still works unchanged
//!      (recap persisted + embedded at close) — regression guard.

use super::*;
use crate::openhuman::agent::harness::archivist::ArchivistHook;
use crate::openhuman::agent::hooks::{PostTurnHook as _, TurnContext};
use crate::openhuman::context::summarizer::{Summarizer, SummaryStats};
use crate::openhuman::inference::provider::{ChatMessage, ConversationMessage};
use crate::openhuman::memory::chat::ChatPrompt;
use crate::openhuman::memory_store::{fts5, segments as seg};
use anyhow::Result;
use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::Connection;
use std::sync::Arc;

// ── Shared test infrastructure ────────────────────────────────────────────────

fn setup_conn() -> Arc<Mutex<Connection>> {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(fts5::EPISODIC_INIT_SQL).unwrap();
    conn.execute_batch(seg::SEGMENTS_INIT_SQL).unwrap();
    conn.execute_batch(crate::openhuman::memory_store::events::EVENTS_INIT_SQL)
        .unwrap();
    conn.execute_batch(crate::openhuman::memory_store::profile::PROFILE_INIT_SQL)
        .unwrap();
    Arc::new(Mutex::new(conn))
}

/// Stub ChatProvider always returns a fixed recap string.
struct StubChatProvider;

#[async_trait]
impl crate::openhuman::memory::chat::ChatProvider for StubChatProvider {
    fn name(&self) -> &str {
        "stub:test"
    }
    async fn chat_for_json(&self, _prompt: &ChatPrompt) -> Result<String> {
        Ok("rolling recap: discussed memory safety in Rust".to_string())
    }
    async fn chat_for_text(&self, _prompt: &ChatPrompt) -> Result<String> {
        Ok("rolling recap: discussed memory safety in Rust".to_string())
    }
}

/// Stub ChatProvider that always fails — simulates LLM unavailability.
struct FailingChatProvider;

#[async_trait]
impl crate::openhuman::memory::chat::ChatProvider for FailingChatProvider {
    fn name(&self) -> &str {
        "stub:failing"
    }
    async fn chat_for_json(&self, _prompt: &ChatPrompt) -> Result<String> {
        anyhow::bail!("stub LLM unavailable")
    }
    async fn chat_for_text(&self, _prompt: &ChatPrompt) -> Result<String> {
        anyhow::bail!("stub LLM unavailable")
    }
}

/// Stub Embedder returns a fixed unit vector.
struct StubEmbedder;

#[async_trait]
impl crate::openhuman::memory_tree::score::embed::Embedder for StubEmbedder {
    fn name(&self) -> &'static str {
        "stub-embedder-v1"
    }
    async fn embed(&self, _text: &str) -> Result<Vec<f32>> {
        Ok(vec![0.5_f32, 0.5, 0.5, 0.5])
    }
}

/// Inner mock summarizer that records call count and always succeeds.
struct RecordingSummarizer {
    calls: std::sync::Mutex<usize>,
}

impl RecordingSummarizer {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: std::sync::Mutex::new(0),
        })
    }
    fn call_count(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

#[async_trait]
impl Summarizer for RecordingSummarizer {
    async fn summarize(
        &self,
        history: &mut Vec<ConversationMessage>,
        _model: &str,
    ) -> Result<SummaryStats> {
        *self.calls.lock().unwrap() += 1;
        // Replace history with a single "inner fallback" message.
        let removed = history.len();
        history.clear();
        history.push(ConversationMessage::Chat(ChatMessage::system(
            "inner fallback summary",
        )));
        Ok(SummaryStats {
            messages_removed: removed,
            approx_tokens_freed: 500,
            summary_chars: 22,
        })
    }
}

fn user(s: &str) -> ConversationMessage {
    ConversationMessage::Chat(ChatMessage::user(s))
}

// ── Test 1: rolling recap does NOT close the segment or write summary/embedding

/// `rolling_segment_recap` must produce a non-empty string from the open
/// segment's entries without closing it, writing `segment_set_summary`, or
/// producing an embedding row.
#[tokio::test]
async fn rolling_recap_does_not_close_segment_or_write_summary_or_embedding() {
    let conn = setup_conn();
    let hook = Arc::new(ArchivistHook::new_with_stubs(
        conn.clone(),
        Arc::new(StubChatProvider),
        Arc::new(StubEmbedder),
    ));

    let session = "p15-rolling-no-close";

    // Write two turns into the open segment (no boundary fires).
    for i in 1..=2u64 {
        hook.on_turn_complete(&TurnContext {
            user_message: format!("Turn {i} about Rust memory safety"),
            assistant_response: format!("Answer {i} about ownership"),
            tool_calls: vec![],
            turn_duration_ms: 50,
            session_id: Some(session.into()),
            agent_id: None,
            entrypoint: None,
            iteration_count: i as usize,
        })
        .await
        .unwrap();
    }

    // Verify the segment is still open before calling rolling_segment_recap.
    let open_before = seg::open_segment_for_session(&conn, session).unwrap();
    assert!(
        open_before.is_some(),
        "Expected an open segment after 2 turns (no boundary should have fired)"
    );

    // Call rolling_segment_recap — must NOT close the segment or write DB state.
    let recap = hook.rolling_segment_recap(session).await;
    assert!(
        recap.is_some(),
        "Expected rolling_segment_recap to return Some for an open segment with entries"
    );
    let recap_text = recap.unwrap();
    assert!(
        !recap_text.is_empty(),
        "Expected non-empty recap from rolling_segment_recap"
    );

    // Segment must STILL be open after the call.
    let open_after = seg::open_segment_for_session(&conn, session).unwrap();
    assert!(
        open_after.is_some(),
        "Segment must still be open after rolling_segment_recap (no close side-effect)"
    );

    let seg_id = open_after.unwrap().segment_id;

    // No summary must have been written.
    let all_segs = seg::segments_by_namespace(&conn, "global", 10).unwrap();
    let our_seg = all_segs.iter().find(|s| s.segment_id == seg_id).unwrap();
    assert!(
        our_seg.summary.is_none() || our_seg.summary.as_deref() == Some(""),
        "segment_set_summary must NOT have been called by rolling_segment_recap; \
         found: {:?}",
        our_seg.summary
    );

    // No embedding must have been written.
    let embedding = seg::segment_embedding_get(&conn, &seg_id, "stub-embedder-v1").unwrap();
    assert!(
        embedding.is_none(),
        "No embedding must be written by rolling_segment_recap \
         (finalize-only invariant); found an embedding for segment={seg_id}"
    );
}

// ── Test 2: compaction uses recap text, not inner summarizer ─────────────────

/// When the rolling recap is available, `SegmentRecapSummarizer::summarize`
/// must use it as the replacement text — the inner summarizer must NOT be
/// called and the history head must contain `[segment-recap]`.
#[tokio::test]
async fn compaction_uses_recap_text_not_inner_summarizer() {
    let conn = setup_conn();
    let hook = Arc::new(ArchivistHook::new_with_stubs(
        conn.clone(),
        Arc::new(StubChatProvider),
        Arc::new(StubEmbedder),
    ));

    let session = "p15-compaction-recap";

    // Write one turn so the open segment has entries.
    hook.on_turn_complete(&TurnContext {
        user_message: "Tell me about Rust ownership".into(),
        assistant_response: "Ownership prevents data races.".into(),
        tool_calls: vec![],
        turn_duration_ms: 50,
        session_id: Some(session.into()),
        agent_id: None,
        entrypoint: None,
        iteration_count: 1,
    })
    .await
    .unwrap();

    let inner = RecordingSummarizer::new();
    let recap_summ = SegmentRecapSummarizer::new(
        Arc::clone(&hook),
        session.to_string(),
        inner.clone() as Arc<dyn Summarizer>,
    )
    .with_keep_recent(1); // compact everything except the last message

    // Build a history that exceeds keep_recent so summarization fires.
    let mut history = vec![
        user("message 1 (will be evicted)"),
        user("message 2 (will be evicted)"),
        user("message 3 — preserved tail"),
    ];

    let stats = recap_summ
        .summarize(&mut history, "test-model")
        .await
        .expect("summarize must succeed");

    // Inner summarizer must NOT have been called.
    assert_eq!(
        inner.call_count(),
        0,
        "Inner summarizer must not be called when rolling recap is available"
    );

    // Stats must reflect a non-zero reduction.
    assert!(
        stats.messages_removed > 0,
        "Expected some messages to be removed; got 0"
    );

    // The new head message must contain the recap text (not the inner fallback).
    assert_eq!(
        history.len(),
        2,
        "Expected summary message + 1 preserved tail; got {}",
        history.len()
    );
    match &history[0] {
        ConversationMessage::Chat(m) => {
            assert!(
                m.content.contains("[segment-recap]"),
                "Expected summary message to contain '[segment-recap]'; got: {:?}",
                m.content
            );
            // Must also contain the actual recap text from the stub.
            assert!(
                m.content.contains("rolling recap"),
                "Expected summary to contain recap text from stub provider; got: {:?}",
                m.content
            );
        }
        other => panic!("Expected Chat message for summary, got: {:?}", other),
    }

    // Tail must be preserved verbatim.
    match &history[1] {
        ConversationMessage::Chat(m) => {
            assert_eq!(m.content, "message 3 — preserved tail");
        }
        other => panic!("Expected Chat tail message, got: {:?}", other),
    }
}

// ── Test 3: soft-fallback when archivist absent ───────────────────────────────

/// When the archivist is disabled (no conn), `rolling_segment_recap` returns
/// `None` and `SegmentRecapSummarizer` must fall back to the inner summarizer.
/// The prompt must remain bounded (no panic, no over-budget, no error).
#[tokio::test]
async fn soft_fallback_when_archivist_absent() {
    // Use a disabled archivist (no SQLite connection).
    let disabled_hook = Arc::new(ArchivistHook::disabled());

    let inner = RecordingSummarizer::new();
    let recap_summ = SegmentRecapSummarizer::new(
        disabled_hook,
        "no-session".to_string(),
        inner.clone() as Arc<dyn Summarizer>,
    )
    .with_keep_recent(1);

    let mut history = vec![user("evict me"), user("evict me too"), user("keep me")];

    let stats = recap_summ
        .summarize(&mut history, "test-model")
        .await
        .expect("summarize must not return Err — soft-fallback guarantees boundedness");

    // Inner summarizer must have been called exactly once.
    assert_eq!(
        inner.call_count(),
        1,
        "Inner summarizer must be called when archivist is absent"
    );

    // History must be bounded (inner replaced it).
    assert_eq!(
        history.len(),
        1,
        "Inner summarizer must have reduced the history"
    );
    match &history[0] {
        ConversationMessage::Chat(m) => {
            assert_eq!(m.content, "inner fallback summary");
        }
        _ => panic!("Expected system summary from inner summarizer"),
    }

    // Stats must be valid (not zeroes — inner mock returns non-zero values).
    assert_eq!(stats.approx_tokens_freed, 500);
}

// ── Test 4: soft-fallback when LLM stub fails ────────────────────────────────

/// Tier 3 — the bookend heuristic stub must NEVER become live compaction
/// text. Here there is no chat provider configured, so `summarize_entries`
/// produces only `segments::fallback_summary` (bookend stub) with
/// `produced_by_llm = false`. `rolling_segment_recap` must therefore return
/// `None`, and `SegmentRecapSummarizer` falls back to the inner summarizer.
/// (Option A: only a genuine summariser-produced recap may be compaction.)
#[tokio::test]
async fn bookend_stub_never_becomes_compaction_falls_back_to_inner() {
    let conn = setup_conn();
    // `ArchivistHook::new` leaves `chat_provider = None` → summarize_entries
    // takes the no-provider branch → bookend stub, produced_by_llm = false.
    let hook = Arc::new(ArchivistHook::new(conn.clone(), true));

    let session = "p15-bookend-stub-none";

    hook.on_turn_complete(&TurnContext {
        user_message: "Hello, what is Rust?".into(),
        assistant_response: "Rust is a systems language.".into(),
        tool_calls: vec![],
        turn_duration_ms: 50,
        session_id: Some(session.into()),
        agent_id: None,
        entrypoint: None,
        iteration_count: 1,
    })
    .await
    .unwrap();

    // Only the bookend stub is available → MUST be None.
    let recap = hook.rolling_segment_recap(session).await;
    assert!(
        recap.is_none(),
        "rolling_segment_recap must return None when only the bookend stub \
         is available — the stub must never be live compaction text"
    );

    let inner = RecordingSummarizer::new();
    let recap_summ = SegmentRecapSummarizer::new(
        Arc::clone(&hook),
        session.to_string(),
        inner.clone() as Arc<dyn Summarizer>,
    )
    .with_keep_recent(1);

    let mut history = vec![user("evict"), user("evict2"), user("keep")];
    recap_summ
        .summarize(&mut history, "test-model")
        .await
        .expect("must not panic — inner summarizer is the safety net");

    assert_eq!(
        inner.call_count(),
        1,
        "Inner summarizer must run when only the bookend stub is available \
         (stub must never be live compaction text)"
    );
}

/// Tier 2 — when a chat provider IS configured but its call fails,
/// `LlmSummariser`'s soft-fallback yields an *inert clipped-content* recap
/// (the real conversation text, truncated — NOT the bookend stub). Per
/// Option A this is acceptable as live compaction text (real content,
/// strictly better than no compaction), so `rolling_segment_recap` returns
/// `Some` and the inner summarizer is NOT used.
#[tokio::test]
async fn failing_provider_yields_inert_clipped_recap_used_as_compaction() {
    let conn = setup_conn();
    let hook = Arc::new(ArchivistHook::new_with_stubs(
        conn.clone(),
        Arc::new(FailingChatProvider),
        Arc::new(StubEmbedder),
    ));

    let session = "p15-inert-clipped-kept";

    hook.on_turn_complete(&TurnContext {
        user_message: "Explain ownership in Rust in detail.".into(),
        assistant_response: "Ownership means each value has a single owner; \
                             borrows are checked at compile time."
            .into(),
        tool_calls: vec![],
        turn_duration_ms: 50,
        session_id: Some(session.into()),
        agent_id: None,
        entrypoint: None,
        iteration_count: 1,
    })
    .await
    .unwrap();

    // Provider present but failing → summarize_entries returns a non-LLM
    // fallback, which rolling_segment_recap must treat as unavailable for
    // live compaction.
    let recap = hook.rolling_segment_recap(session).await;
    assert!(
        recap.is_none(),
        "Non-LLM fallback recap text must not be used as live compaction input"
    );

    let inner = RecordingSummarizer::new();
    let recap_summ = SegmentRecapSummarizer::new(
        Arc::clone(&hook),
        session.to_string(),
        inner.clone() as Arc<dyn Summarizer>,
    )
    .with_keep_recent(1);

    let mut history = vec![user("evict"), user("evict2"), user("keep")];
    recap_summ
        .summarize(&mut history, "test-model")
        .await
        .expect("must not panic");

    assert_eq!(
        inner.call_count(),
        1,
        "Inner summarizer must run when rolling recap is unavailable after \
         provider failure"
    );
}

// ── Test 5: flag off → inner summarizer runs, Phase 1.5 absent ───────────────

/// When `unified_compaction_enabled = false`, the `SegmentRecapSummarizer`
/// is NOT instantiated — the builder uses `ProviderSummarizer` directly.
/// This test verifies the flag-off path via the `RecordingSummarizer` fallback:
/// a `SegmentRecapSummarizer` with an otherwise-healthy archivist must still
/// fall through to the inner summarizer if the session has no entries yet
/// (no open segment entries → recap = None → inner).
#[tokio::test]
async fn no_entries_returns_none_and_inner_summarizer_fires() {
    let conn = setup_conn();
    // Archivist with no turns written — no open segment, no entries.
    let hook = Arc::new(ArchivistHook::new_with_stubs(
        conn.clone(),
        Arc::new(StubChatProvider),
        Arc::new(StubEmbedder),
    ));

    let inner = RecordingSummarizer::new();
    let recap_summ = SegmentRecapSummarizer::new(
        Arc::clone(&hook),
        "empty-session".to_string(),
        inner.clone() as Arc<dyn Summarizer>,
    )
    .with_keep_recent(1);

    let mut history = vec![user("a"), user("b"), user("c")];
    let _stats = recap_summ
        .summarize(&mut history, "test-model")
        .await
        .expect("must not error");

    // With no open segment, rolling_segment_recap returns None →
    // inner summarizer fires.
    assert_eq!(
        inner.call_count(),
        1,
        "Inner summarizer must run when session has no open segment entries"
    );
}

// ── Test 6: Phase 1 regression guard ─────────────────────────────────────────

/// `on_segment_closed` (finalize path) still works after Phase 1.5 refactor:
/// the shared `summarize_entries` helper must produce the same result, and
/// `segment_set_summary` + embedding must still fire at close time.
#[tokio::test]
async fn phase1_finalize_path_still_persists_summary_and_embedding() {
    let conn = setup_conn();
    let hook = Arc::new(ArchivistHook::new_with_stubs(
        conn.clone(),
        Arc::new(StubChatProvider),
        Arc::new(StubEmbedder),
    ));

    let session = "p15-finalize-regression";

    // Two turns in the first segment.
    for i in 1..=2u64 {
        hook.on_turn_complete(&TurnContext {
            user_message: format!("Rust turn {i}"),
            assistant_response: format!("Answer {i}"),
            tool_calls: vec![],
            turn_duration_ms: 50,
            session_id: Some(session.into()),
            agent_id: None,
            entrypoint: None,
            iteration_count: i as usize,
        })
        .await
        .unwrap();
    }

    // Force-flush to trigger on_segment_closed (finalize path).
    hook.flush_open_segment(session).await;

    // The closed segment must have a summary.
    let all_segs = seg::segments_by_namespace(&conn, "global", 10).unwrap();
    let flushed = all_segs
        .iter()
        .find(|s| {
            s.session_id == session && s.summary.as_ref().map(|s| !s.is_empty()).unwrap_or(false)
        })
        .expect("Expected flushed segment to have a non-empty summary after Phase 1.5 refactor");

    let summary = flushed.summary.as_ref().unwrap();
    // The stub always returns "rolling recap: discussed memory safety in Rust".
    assert!(
        summary.contains("rolling recap"),
        "Expected summary to contain stub recap text after finalize; got: {summary:?}"
    );

    // Embedding must also still be written (finalize-only invariant).
    let embedding =
        seg::segment_embedding_get(&conn, &flushed.segment_id, "stub-embedder-v1").unwrap();
    assert!(
        embedding.is_some(),
        "Expected finalize-time embedding to still be written after Phase 1.5 refactor"
    );
}
