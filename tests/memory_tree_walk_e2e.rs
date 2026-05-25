//! E2E tests for the `memory_tree_walk` agentic tool.
//!
//! These tests exercise `run_walk` end-to-end against a real HTTP mock
//! LLM server (wiremock) to prove that:
//!   - The `OpenAiCompatibleProvider` → `run_walk` chain works over HTTP.
//!   - Tool-call XML parsing, trace assembly, and stop-reason detection
//!     all behave correctly with per-turn scripted LLM responses.
//!   - Turn-cap enforcement fires correctly.
//!   - Unknown node descends are handled gracefully.
//!
//! Tree nodes are seeded directly via `store::write_node` so the tests
//! are isolated from the summariser pipeline and focus purely on walk.
//!
//! Run with:
//!   cargo test --test memory_tree_walk_e2e
//! or via the project wrapper:
//!   bash scripts/test-rust-with-mock.sh --test memory_tree_walk_e2e

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

use chrono::Utc;
use serde_json::json;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::inference::provider::compatible::{
    AuthStyle, OpenAiCompatibleProvider,
};
use openhuman_core::openhuman::memory_tree::tools::walk::{run_walk, WalkOptions, WalkStopReason};
use openhuman_core::openhuman::memory_tree::tree_runtime::store::write_node;
use openhuman_core::openhuman::memory_tree::tree_runtime::types::{
    derive_parent_id, estimate_tokens, level_from_node_id, TreeNode,
};

// ── Environment serialisation lock ──────────────────────────────────────────
//
// `OPENHUMAN_WORKSPACE` is a process-global env var.  Tests that set it
// must serialise with this lock.

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    let m = ENV_LOCK.get_or_init(|| Mutex::new(()));
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

// ── Per-turn sequential mock responder ──────────────────────────────────────
//
// `ScriptedResponder` pops one canned OpenAI-compatible JSON response from its
// queue on each `respond` call.  This allows a single wiremock `Mock` to hand
// back turn-1, turn-2, turn-3 responses in sequence.
//
// The queue is `Arc<Mutex<VecDeque<String>>>` so it can be shared between the
// `Mock` registration (which clones the arc) and the test body for inspection.

#[derive(Clone)]
struct ScriptedResponder {
    queue: Arc<Mutex<VecDeque<String>>>,
    call_count: Arc<Mutex<usize>>,
}

impl ScriptedResponder {
    /// Create a new responder pre-loaded with `content_strings`.
    ///
    /// Each string becomes the `choices[0].message.content` of an
    /// OpenAI-compatible non-streaming response.
    fn new(content_strings: Vec<&str>) -> Self {
        log::debug!(
            "[memory_tree_walk_e2e] ScriptedResponder created with {} turns",
            content_strings.len()
        );
        let queue: VecDeque<String> = content_strings.iter().map(|s| s.to_string()).collect();
        Self {
            queue: Arc::new(Mutex::new(queue)),
            call_count: Arc::new(Mutex::new(0)),
        }
    }

    fn call_count(&self) -> usize {
        *self.call_count.lock().unwrap()
    }
}

impl Respond for ScriptedResponder {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        let mut queue = self.queue.lock().unwrap();
        let mut count = self.call_count.lock().unwrap();
        *count += 1;
        let current_turn = *count;

        let content = queue
            .pop_front()
            .unwrap_or_else(|| "ScriptedResponder: queue exhausted".to_string());

        log::debug!(
            "[memory_tree_walk_e2e] ScriptedResponder turn={current_turn} content_preview={}",
            &content[..content.len().min(80)]
        );

        let body = json!({
            "id": format!("chatcmpl-walk-e2e-{current_turn}"),
            "object": "chat.completion",
            "created": 1_700_000_000_u64,
            "model": "e2e-walk-model",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": content
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 20,
                "completion_tokens": 10,
                "total_tokens": 30
            }
        });

        ResponseTemplate::new(200).set_body_json(body)
    }
}

// ── Tree helpers ─────────────────────────────────────────────────────────────

/// Build a minimal `Config` pointing at a temp workspace.
fn test_config(tmp: &TempDir) -> Config {
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().join("workspace");
    std::fs::create_dir_all(&cfg.workspace_dir).expect("create workspace dir");
    cfg
}

/// Create a `TreeNode` with the correct level and parent derived from its id.
fn make_node(namespace: &str, node_id: &str, summary: &str, child_count: u32) -> TreeNode {
    let level = level_from_node_id(node_id);
    let parent_id = derive_parent_id(node_id);
    let ts = Utc::now();
    TreeNode {
        node_id: node_id.to_string(),
        namespace: namespace.to_string(),
        level,
        parent_id,
        summary: summary.to_string(),
        token_count: estimate_tokens(summary),
        child_count,
        created_at: ts,
        updated_at: ts,
        metadata: None,
    }
}

/// Seed a 3-level tree:
///   root → 2024 (year) → 2024/01 (month leaf)
///
/// Returns the node ids for easy reference in tests.
fn seed_tree(cfg: &Config, ns: &str) -> (&'static str, &'static str, &'static str) {
    let root_id = "root";
    let year_id = "2024";
    let month_id = "2024/01";

    write_node(
        cfg,
        &make_node(ns, root_id, "All-time summary: project activity 2024.", 1),
    )
    .expect("write root node");

    write_node(
        cfg,
        &make_node(
            ns,
            year_id,
            "Year 2024: major deployment cycle, shipped v1 release.",
            1,
        ),
    )
    .expect("write year node");

    write_node(
        cfg,
        &make_node(
            ns,
            month_id,
            "January 2024: user discussed deployment X on the Slack channel.",
            0,
        ),
    )
    .expect("write month node");

    log::debug!(
        "[memory_tree_walk_e2e] seeded tree namespace={ns} root={root_id} year={year_id} month={month_id}"
    );

    (root_id, year_id, month_id)
}

// ── Provider helper ───────────────────────────────────────────────────────────

/// Build an `OpenAiCompatibleProvider` pointing at `wiremock_uri/v1`.
fn make_provider(wiremock_uri: &str) -> OpenAiCompatibleProvider {
    log::debug!(
        "[memory_tree_walk_e2e] building provider base_url={}/v1",
        wiremock_uri
    );
    OpenAiCompatibleProvider::new(
        "e2e-walk-test",
        &format!("{}/v1", wiremock_uri),
        Some("test-key"),
        AuthStyle::Bearer,
    )
}

// ── Test 1: walks_descend_fetch_answer ───────────────────────────────────────

/// Happy-path walk: descend → fetch_leaves → answer in 3 turns.
///
/// Validates:
/// - `WalkStopReason::Answered`
/// - `answer` contains expected text
/// - trace has 3 steps with correct action names
/// - `turns_used == 3`
/// - mock LLM received exactly 3 HTTP calls
#[tokio::test]
async fn walks_descend_fetch_answer() {
    let _lock = env_lock();

    log::debug!("[memory_tree_walk_e2e] test=walks_descend_fetch_answer starting");

    // ── Start wiremock ──
    let server = MockServer::start().await;
    log::debug!(
        "[memory_tree_walk_e2e] wiremock listening at {}",
        server.uri()
    );

    // ── Seed tree ──
    let tmp = TempDir::new().expect("tempdir");
    let cfg = test_config(&tmp);
    let ns = "e2e-walk-test";
    let (_, year_id, month_id) = seed_tree(&cfg, ns);

    // ── Script 3 turns ──
    // Turn 1: descend into the year node.
    // Turn 2: fetch_leaves on the month node.
    // Turn 3: answer.
    let turn1 = format!(
        r#"<tool_call>{{"name":"descend","arguments":{{"node_id":"{year_id}"}}}}</tool_call>"#
    );
    let turn2 = format!(
        r#"<tool_call>{{"name":"fetch_leaves","arguments":{{"node_id":"{month_id}"}}}}</tool_call>"#
    );
    let turn3 =
        r#"<tool_call>{"name":"answer","arguments":{"text":"The user discussed deployment X"}}</tool_call>"#.to_string();

    let responder = ScriptedResponder::new(vec![&turn1, &turn2, &turn3]);
    let responder_clone = responder.clone();

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(responder)
        .mount(&server)
        .await;

    // ── Run walk ──
    let provider = make_provider(&server.uri());
    let opts = WalkOptions {
        max_turns: 6,
        start_node_id: None,
        namespace: ns.to_string(),
        model: Some("e2e-walk-model".into()),
    };

    log::debug!("[memory_tree_walk_e2e] calling run_walk query='deployment query'");
    let outcome = run_walk(
        &cfg,
        &provider,
        "What was discussed about deployment?",
        opts,
    )
    .await
    .expect("run_walk should succeed");

    log::debug!(
        "[memory_tree_walk_e2e] outcome stopped_reason={:?} turns_used={} trace_len={}",
        outcome.stopped_reason,
        outcome.turns_used,
        outcome.trace.len()
    );

    // ── Assertions ──
    assert_eq!(
        outcome.stopped_reason,
        WalkStopReason::Answered,
        "walk should stop with Answered, got {:?}",
        outcome.stopped_reason
    );
    assert!(
        outcome.answer.contains("The user discussed deployment X"),
        "answer should contain expected text, got: {}",
        outcome.answer
    );
    assert_eq!(
        outcome.turns_used, 3,
        "expected 3 turns, got {}",
        outcome.turns_used
    );
    assert_eq!(
        outcome.trace.len(),
        3,
        "expected trace of 3 steps, got {}",
        outcome.trace.len()
    );
    assert_eq!(
        outcome.trace[0].action, "descend",
        "step 0 should be descend"
    );
    assert_eq!(
        outcome.trace[1].action, "fetch_leaves",
        "step 1 should be fetch_leaves"
    );
    assert_eq!(outcome.trace[2].action, "answer", "step 2 should be answer");

    // Verify LLM was called exactly 3 times over HTTP.
    let llm_calls = responder_clone.call_count();
    assert_eq!(
        llm_calls, 3,
        "LLM mock should have received exactly 3 HTTP requests, got {llm_calls}"
    );

    log::debug!("[memory_tree_walk_e2e] test=walks_descend_fetch_answer PASSED");
}

// ── Test 2: respects_max_turns_cap_with_mock ─────────────────────────────────

/// Turn-cap enforcement: the mock always returns `descend` (never `answer`),
/// so the walk must stop at `max_turns` with `MaxTurnsReached`.
///
/// Validates:
/// - `WalkStopReason::MaxTurnsReached`
/// - `turns_used == max_turns` (3)
/// - fallback answer contains "converge" or similar marker
/// - mock received exactly 3 HTTP calls (== max_turns)
#[tokio::test]
async fn respects_max_turns_cap_with_mock() {
    let _lock = env_lock();

    log::debug!("[memory_tree_walk_e2e] test=respects_max_turns_cap_with_mock starting");

    let server = MockServer::start().await;
    log::debug!(
        "[memory_tree_walk_e2e] wiremock listening at {}",
        server.uri()
    );

    let tmp = TempDir::new().expect("tempdir");
    let cfg = test_config(&tmp);
    let ns = "e2e-walk-cap-test";
    let (_, year_id, _) = seed_tree(&cfg, ns);

    // Always descend (never answer) — more entries than max_turns so the cap fires.
    let forever_descend = format!(
        r#"<tool_call>{{"name":"descend","arguments":{{"node_id":"{year_id}"}}}}</tool_call>"#
    );

    let responder = ScriptedResponder::new(vec![
        &forever_descend,
        &forever_descend,
        &forever_descend,
        &forever_descend,
        &forever_descend,
    ]);
    let responder_clone = responder.clone();

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(responder)
        .mount(&server)
        .await;

    let provider = make_provider(&server.uri());
    let opts = WalkOptions {
        max_turns: 3,
        start_node_id: None,
        namespace: ns.to_string(),
        model: Some("e2e-walk-model".into()),
    };

    log::debug!("[memory_tree_walk_e2e] calling run_walk with max_turns=3");
    let outcome = run_walk(&cfg, &provider, "infinite loop query", opts)
        .await
        .expect("run_walk should succeed (not error)");

    log::debug!(
        "[memory_tree_walk_e2e] outcome stopped_reason={:?} turns_used={} trace_len={}",
        outcome.stopped_reason,
        outcome.turns_used,
        outcome.trace.len()
    );

    assert_eq!(
        outcome.stopped_reason,
        WalkStopReason::MaxTurnsReached,
        "walk should stop with MaxTurnsReached, got {:?}",
        outcome.stopped_reason
    );
    assert_eq!(
        outcome.turns_used, 3,
        "turns_used should be max_turns=3, got {}",
        outcome.turns_used
    );
    assert!(
        outcome.answer.to_lowercase().contains("converge")
            || outcome.answer.to_lowercase().contains("turn limit")
            || outcome.answer.to_lowercase().contains("could not"),
        "fallback answer should indicate failure to converge, got: {}",
        outcome.answer
    );

    // Mock should have been called exactly max_turns (3) times.
    let llm_calls = responder_clone.call_count();
    assert_eq!(
        llm_calls, 3,
        "LLM mock should have received exactly 3 HTTP requests (max_turns), got {llm_calls}"
    );

    log::debug!("[memory_tree_walk_e2e] test=respects_max_turns_cap_with_mock PASSED");
}

// ── Test 3: handles_unknown_node_gracefully ───────────────────────────────────

/// Unknown-node recovery: turn 1 descends into a non-existent node;
/// the walk reports "unknown node" in the trace but continues.
/// Turn 2 answers, so the walk completes with `Answered`.
///
/// Validates:
/// - `WalkStopReason::Answered`
/// - `trace[0].result_preview` contains "unknown node"
/// - `trace.len() == 2`
/// - answer from turn 2 is preserved
#[tokio::test]
async fn handles_unknown_node_gracefully() {
    let _lock = env_lock();

    log::debug!("[memory_tree_walk_e2e] test=handles_unknown_node_gracefully starting");

    let server = MockServer::start().await;
    log::debug!(
        "[memory_tree_walk_e2e] wiremock listening at {}",
        server.uri()
    );

    let tmp = TempDir::new().expect("tempdir");
    let cfg = test_config(&tmp);
    let ns = "e2e-walk-unknown-test";
    seed_tree(&cfg, ns);

    // Turn 1: descend into a node that does not exist.
    let turn1 =
        r#"<tool_call>{"name":"descend","arguments":{"node_id":"does_not_exist"}}</tool_call>"#;
    // Turn 2: answer (walk continues after bad descend).
    let turn2 = r#"<tool_call>{"name":"answer","arguments":{"text":"I gave up: node not found"}}</tool_call>"#;

    let responder = ScriptedResponder::new(vec![turn1, turn2]);
    let responder_clone = responder.clone();

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(responder)
        .mount(&server)
        .await;

    let provider = make_provider(&server.uri());
    let opts = WalkOptions {
        max_turns: 6,
        start_node_id: None,
        namespace: ns.to_string(),
        model: Some("e2e-walk-model".into()),
    };

    log::debug!("[memory_tree_walk_e2e] calling run_walk with unknown-node script");
    let outcome = run_walk(&cfg, &provider, "find nonexistent data", opts)
        .await
        .expect("run_walk should succeed");

    log::debug!(
        "[memory_tree_walk_e2e] outcome stopped_reason={:?} turns_used={} trace_len={}",
        outcome.stopped_reason,
        outcome.turns_used,
        outcome.trace.len()
    );

    // Walk should complete despite the bad descend.
    assert_eq!(
        outcome.stopped_reason,
        WalkStopReason::Answered,
        "walk should eventually answer, got {:?}",
        outcome.stopped_reason
    );

    assert_eq!(
        outcome.trace.len(),
        2,
        "expected 2 trace steps, got {}",
        outcome.trace.len()
    );

    // The first step's result_preview should indicate the unknown node.
    let step0_preview = &outcome.trace[0].result_preview;
    assert!(
        step0_preview.contains("unknown node"),
        "step 0 result_preview should contain 'unknown node', got: {step0_preview}"
    );

    // The final answer should be from turn 2.
    assert!(
        outcome.answer.contains("I gave up"),
        "answer should contain turn-2 text, got: {}",
        outcome.answer
    );

    // 2 HTTP calls made.
    let llm_calls = responder_clone.call_count();
    assert_eq!(
        llm_calls, 2,
        "LLM mock should have received exactly 2 HTTP requests, got {llm_calls}"
    );

    log::debug!("[memory_tree_walk_e2e] test=handles_unknown_node_gracefully PASSED");
}
