//! The memory tool's reads: recall returns the engine's evidence, and recall
//! and fetch leave the calling thread out.

use super::*;
use crate::memory::ops;
use crate::memory::test_fixtures::{bind_reference, config_in};
use crate::memory::types::RecallParams;
use std::sync::Arc;
use tinymemory_api::conformance::ReferenceEngine;
use tinymemory_api::{EngineDescriptor, MemoryEngine};

const EVIDENCE_TOKEN_BUDGET: usize = 2_500;

struct FixedRecallEngine {
    inner: ReferenceEngine,
    answer: String,
}

#[async_trait::async_trait]
impl MemoryEngine for FixedRecallEngine {
    fn descriptor(&self) -> &EngineDescriptor {
        self.inner.descriptor()
    }

    async fn health(&self) -> tinymemory_api::EngineHealth {
        self.inner.health().await
    }

    async fn recall(
        &self,
        _request: tinymemory_api::RecallRequest,
    ) -> tinymemory_api::Result<tinymemory_api::RecallAnswer> {
        Ok(tinymemory_api::RecallAnswer {
            answer: self.answer.clone(),
            citations: Vec::new(),
            model: None,
        })
    }

    async fn fetch(
        &self,
        request: tinymemory_api::FetchRequest,
    ) -> tinymemory_api::Result<tinymemory_api::FetchPage> {
        self.inner.fetch(request).await
    }

    async fn store(
        &self,
        item: tinymemory_api::StoreItem,
    ) -> tinymemory_api::Result<tinymemory_api::StoreReceipt> {
        self.inner.store(item).await
    }

    async fn forget(
        &self,
        target: tinymemory_api::ForgetTarget,
    ) -> tinymemory_api::Result<tinymemory_api::ForgetReport> {
        self.inner.forget(target).await
    }

    async fn list(
        &self,
        request: tinymemory_api::ListRequest,
    ) -> tinymemory_api::Result<tinymemory_api::ListPage> {
        self.inner.list(request).await
    }
}

fn native_capped_evidence(evidence: &str) -> String {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    crate::memory::engine::install_test_engine(
        &config.workspace_dir,
        Arc::new(FixedRecallEngine {
            inner: ReferenceEngine::new(),
            answer: evidence.to_string(),
        }),
    );
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let result = MemoryTool::new(Arc::new(config))
                .execute(json!({"action":"recall","question":"evidence"}))
                .await
                .unwrap();
            assert!(!result.is_error, "{}", result.text());
            serde_json::from_str::<Value>(&result.text()).unwrap()["evidence"]
                .as_str()
                .unwrap()
                .to_string()
        })
}

fn facts() -> CallFacts {
    CallFacts {
        thread_id: Some("thread-tool-1".into()),
        agent_id: Some("orchestrator".into()),
        ..CallFacts::of(
            &Config::default(),
            &tinymemory_tools::layout::identity::Identity::agent("orchestrator"),
        )
    }
}

fn text(result: &ToolResult) -> String {
    result.text()
}

/// A fetch result's page as the model reads it, and how many items it holds.
fn page(result: &ToolResult) -> (String, u64) {
    assert!(!result.is_error, "{}", text(result));
    let out: Value = serde_json::from_str(&text(result)).unwrap();
    (
        out["matches"].as_str().unwrap().to_string(),
        out["items"].as_u64().unwrap(),
    )
}

/// A one-turn conversation in `thread`, where `facts` reads and writes.
fn turn_in(facts: &CallFacts, thread: &str, text: &str) -> tinymemory_api::StoreItem {
    turn_at(facts, thread, 0, text)
}

/// Turn `index` of `thread`, where `facts` reads and writes.
fn turn_at(facts: &CallFacts, thread: &str, index: u32, text: &str) -> tinymemory_api::StoreItem {
    tinymemory_api::StoreItem::Conversation {
        turns: vec![tinymemory_api::Turn::new(tinymemory_api::Role::User, text)],
        meta: tinymemory_api::MemoryMeta {
            namespace: facts.namespace.clone(),
            thread_id: Some(thread.to_string()),
            turns: Some(tinymemory_api::TurnRange {
                first: index,
                last: index,
            }),
            ..tinymemory_api::MemoryMeta::default()
        },
    }
}

/// A long thread whose turns before `from_turn` were compacted out of the
/// prompt: turn 2 (compacted) and turn 8 (still in the prompt).
async fn compacted_thread(engine: &impl MemoryEngine, from_turn: u32) -> CallFacts {
    // A thread of its own: recall records citations per thread id, process
    // wide, and tests run in parallel.
    let mut facts = CallFacts {
        thread_id: Some("thread-compacted".into()),
        ..facts()
    };
    let live = facts.thread_id.clone().unwrap();
    engine
        .store(turn_at(
            &facts,
            &live,
            2,
            "I liked the oolong tea from Taiwan",
        ))
        .await
        .unwrap();
    engine
        .store(turn_at(&facts, &live, 8, "which oolong tea did I like"))
        .await
        .unwrap();
    facts.in_prompt = Some(ThreadWindow {
        thread_id: live,
        from_turn,
    });
    facts
}

#[tokio::test]
async fn recall_returns_evidence_and_leaves_the_calling_thread_out() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let facts = facts();
    let live = facts.thread_id.clone().unwrap();
    engine
        .store(turn_in(&facts, &live, "which oolong tea did I like"))
        .await
        .unwrap();
    engine
        .store(turn_in(
            &facts,
            "thread-past",
            "I liked the oolong tea from Taiwan",
        ))
        .await
        .unwrap();

    let recalled = run_action(
        &DispatchCtx::ambient(),
        &config,
        &json!({"action": "recall", "question": "oolong tea"}),
        &facts,
    )
    .await;
    assert!(!recalled.is_error, "{}", text(&recalled));
    let out: Value = serde_json::from_str(&text(&recalled)).unwrap();
    let evidence = out["evidence"].as_str().unwrap();
    assert!(evidence.contains("oolong tea from Taiwan"), "{out}");
    assert!(
        !evidence.contains("which oolong tea did I like"),
        "the turn being answered must not come back as a memory: {out}"
    );
    assert_eq!(out["items"], 1);
    assert!(
        out.get("answer").is_none(),
        "recall writes no answer of its own"
    );
    let cites = crate::web_chat::citations::take_turn_citations(&DispatchCtx::ambient(), &live);
    assert_eq!(cites.len(), 1, "{cites:?}");
}

#[tokio::test]
async fn fetch_leaves_the_calling_thread_out() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let facts = facts();
    let live = facts.thread_id.clone().unwrap();
    engine
        .store(turn_in(&facts, &live, "oolong question in this thread"))
        .await
        .unwrap();
    engine
        .store(turn_in(&facts, "thread-past", "oolong answer from before"))
        .await
        .unwrap();

    let fetched = run_action(
        &DispatchCtx::ambient(),
        &config,
        &json!({"action": "fetch", "query": "oolong", "mode": "keyword", "limit": 5}),
        &facts,
    )
    .await;
    let (shown, items) = page(&fetched);
    assert_eq!(items, 1, "{shown}");
    assert!(shown.contains("oolong answer from before"), "{shown}");
    assert!(!shown.contains("oolong question in this thread"), "{shown}");
}

#[tokio::test]
async fn recall_with_nothing_relevant_says_so() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    bind_reference(&config);
    let recalled = run_action(
        &DispatchCtx::ambient(),
        &config,
        &json!({"action": "recall", "question": "anything at all"}),
        &facts(),
    )
    .await;
    assert!(!recalled.is_error, "{}", text(&recalled));
    let out: Value = serde_json::from_str(&text(&recalled)).unwrap();
    assert_eq!(out["evidence"], "");
    assert_eq!(out["note"], "Nothing relevant is stored in memory.");
}

#[tokio::test]
async fn the_rpc_recall_still_answers_and_excludes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let facts = facts();
    engine
        .store(turn_in(
            &facts,
            "thread-tool-1",
            "oolong in the live thread",
        ))
        .await
        .unwrap();
    let view = ops::recall(
        &config,
        RecallParams {
            question: "oolong".into(),
            filter: None,
            limit: None,
            refers_to: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        view.citations.len(),
        1,
        "no thread is excluded outside the tool"
    );
    assert!(
        view.answer.starts_with("From 1 stored items"),
        "{}",
        view.answer
    );
    assert!(
        view.model.is_some(),
        "answer mode reports the answering engine"
    );
}

#[tokio::test]
async fn recall_reads_its_own_threads_compacted_turns_but_not_the_ones_in_the_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let facts = compacted_thread(engine.as_ref(), 6).await;

    let recalled = run_action(
        &DispatchCtx::ambient(),
        &config,
        &json!({"action": "recall", "question": "oolong tea"}),
        &facts,
    )
    .await;
    assert!(!recalled.is_error, "{}", text(&recalled));
    let out: Value = serde_json::from_str(&text(&recalled)).unwrap();
    let evidence = out["evidence"].as_str().unwrap();
    assert!(
        evidence.contains("oolong tea from Taiwan"),
        "a turn compacted out of the prompt is readable again: {out}"
    );
    assert!(
        !evidence.contains("which oolong tea did I like"),
        "a turn still in the prompt must not come back as a memory: {out}"
    );
}

#[tokio::test]
async fn fetch_reads_its_own_threads_compacted_turns_but_not_the_ones_in_the_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let facts = compacted_thread(engine.as_ref(), 6).await;

    let fetched = run_action(
        &DispatchCtx::ambient(),
        &config,
        &json!({"action": "fetch", "query": "oolong", "mode": "keyword", "limit": 5}),
        &facts,
    )
    .await;
    let (shown, items) = page(&fetched);
    assert_eq!(items, 1, "{shown}");
    assert!(shown.contains("oolong tea from Taiwan"), "{shown}");
    assert!(!shown.contains("which oolong tea did I like"), "{shown}");
}

#[tokio::test]
async fn a_window_for_another_thread_leaves_the_whole_calling_thread_out() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let mut facts = compacted_thread(engine.as_ref(), 6).await;
    facts.in_prompt = Some(ThreadWindow {
        thread_id: "thread-elsewhere".into(),
        from_turn: 6,
    });

    let fetched = run_action(
        &DispatchCtx::ambient(),
        &config,
        &json!({"action": "fetch", "query": "oolong", "mode": "keyword", "limit": 5}),
        &facts,
    )
    .await;
    let (shown, items) = page(&fetched);
    assert_eq!(items, 0, "{shown}");
    assert_eq!(shown, "Nothing in memory matches.");
}

#[tokio::test]
async fn fetch_shows_its_top_five_as_dated_lines_and_pages_on_with_its_cursor() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let facts = facts();
    for i in 0..7 {
        let mut item = turn_in(&facts, "thread-past", &format!("oolong note {i}"));
        if let tinymemory_api::StoreItem::Conversation { meta, .. } = &mut item {
            meta.observed_at = Some(
                chrono::DateTime::parse_from_rfc3339("2023-05-21T18:53:00Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc),
            );
        }
        engine.store(item).await.unwrap();
    }

    let fetched = run_action(
        &DispatchCtx::ambient(),
        &config,
        &json!({"action": "fetch", "query": "oolong note", "mode": "keyword"}),
        &facts,
    )
    .await;
    let (shown, items) = page(&fetched);
    assert_eq!(items, 5, "five at a time: {shown}");
    assert!(shown.starts_with("Top 5 matches, best first."), "{shown}");
    assert!(
        shown.contains("[2023-05-21 18:53 UTC] user: oolong note"),
        "{shown}"
    );
    assert!(
        shown.contains("(id "),
        "each match carries the id forget takes: {shown}"
    );
    assert!(!shown.contains("namespace"), "no metadata dump: {shown}");

    // More match: the engine's cursor continues the same search.
    let cursor = serde_json::from_str::<Value>(&text(&fetched)).unwrap()["next_cursor"]
        .as_str()
        .expect("a fetch with more matches returns next_cursor")
        .to_string();
    let next = run_action(
        &DispatchCtx::ambient(),
        &config,
        &json!({"action": "fetch", "query": "oolong note", "mode": "keyword", "cursor": cursor}),
        &facts,
    )
    .await;
    let (rest, items) = page(&next);
    assert_eq!(items, 2, "{rest}");
    assert!(
        serde_json::from_str::<Value>(&text(&next))
            .unwrap()
            .get("next_cursor")
            .is_none(),
        "the last page has no cursor"
    );
    let notes = |text: &str| {
        (0..7)
            .filter(|i| text.contains(&format!("oolong note {i}")))
            .collect::<Vec<_>>()
    };
    let mut seen = notes(&shown);
    seen.extend(notes(&rest));
    seen.sort_unstable();
    assert_eq!(
        seen,
        (0..7).collect::<Vec<_>>(),
        "the two pages hold every note once"
    );

    let more = run_action(
        &DispatchCtx::ambient(),
        &config,
        &json!({"action": "fetch", "query": "oolong note", "mode": "keyword", "limit": 10}),
        &facts,
    )
    .await;
    assert_eq!(
        page(&more).1,
        5,
        "the page size is the host's: a model's limit is ignored"
    );
}

/// `n` entries of `lines` lines each, `width` characters a line, as an
/// engine renders evidence.
fn evidence_of(n: usize, lines: usize, width: usize) -> String {
    (0..n)
        .map(|e| {
            (0..lines)
                .map(|l| {
                    let head = format!("[2023-05-21 01:33 UTC] [user] entry {e} line {l} ");
                    format!("{head}{}", "x".repeat(width.saturating_sub(head.len())))
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

/// Every line of `capped` is a whole line of `evidence`, or one of the cap's
/// own markers.
fn assert_whole_lines(evidence: &str, capped: &str) {
    let source: std::collections::HashSet<&str> = evidence.lines().collect();
    for line in capped.lines() {
        assert!(
            source.contains(line) || line == "[…]" || line.starts_with('('),
            "a line was cut: {line:?}"
        );
    }
}

#[test]
fn short_evidence_is_left_as_it_is() {
    let evidence = evidence_of(3, 2, 80);
    assert_eq!(native_capped_evidence(&evidence), evidence);
}

#[test]
fn long_evidence_keeps_whole_entries_then_whole_lines_and_says_what_was_left_out() {
    // 30 entries of 4 lines of 200 characters: about 6,000 tokens.
    let evidence = evidence_of(30, 4, 200);
    let capped = native_capped_evidence(&evidence);
    let note = capped.rfind("\n\n(").unwrap();
    assert!(
        note <= EVIDENCE_TOKEN_BUDGET * 4,
        "the evidence stays within the budget"
    );
    assert_whole_lines(&evidence, &capped);
    // 12 whole entries (12 × 803 + 11 separators = 9,713 characters), then
    // the 13th cut after its first line.
    assert!(capped.contains("entry 11 line 3"), "{capped}");
    assert!(capped.contains("entry 12 line 0"), "{capped}");
    assert!(!capped.contains("entry 12 line 1"), "{capped}");
    assert!(
        capped.contains("entry 12 line 0 xxx"),
        "the kept line is whole"
    );
    assert!(capped.contains("\n[…]"), "a shortened entry is marked");
    assert!(
        capped.contains("(17 more memory entries left out, the last one shortened,")
            && capped.contains("search again with more specific words"),
        "{}",
        &capped[note..]
    );
}

#[test]
fn a_first_line_longer_than_the_budget_is_kept_whole_never_split() {
    let evidence = evidence_of(2, 1, EVIDENCE_TOKEN_BUDGET * 4 + 500);
    let capped = native_capped_evidence(&evidence);
    assert_whole_lines(&evidence, &capped);
    assert!(capped.starts_with(evidence.lines().next().unwrap()));
    assert!(!capped.contains("[…]"), "the first entry was kept whole");
    assert!(
        capped.contains("(1 more memory entries left out to keep"),
        "{capped}"
    );
}

#[test]
fn the_partly_searched_note_survives_the_cap() {
    let evidence = format!(
        "{}\n\n(Memory was only partly searched: one scope did not answer in time.)",
        evidence_of(30, 4, 200)
    );
    let capped = native_capped_evidence(&evidence);
    assert!(
        capped.ends_with("(Memory was only partly searched: one scope did not answer in time.)")
    );
    assert_whole_lines(&evidence, &capped);
}
