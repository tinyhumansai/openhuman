//! Graph-mechanics tests: full-cycle walk, node ordering (guard after mutations,
//! before END), context-guard eviction threshold, and the loop-continuity
//! property (adversarial state combos never cycle or double-send).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::*;

/// Records the ordered sequence of node operations + every DM sent.
#[derive(Default)]
struct Recorder {
    order: Mutex<Vec<String>>,
    instruct_calls: AtomicUsize,
    compile_calls: AtomicUsize,
    execute_calls: AtomicUsize,
    compress_calls: AtomicUsize,
    world_diff_calls: AtomicUsize,
    evict_calls: AtomicUsize,
    dms: Mutex<Vec<(String, String)>>,
}

impl Recorder {
    fn mark(&self, op: &str) {
        self.order.lock().unwrap().push(op.to_string());
    }
    fn order(&self) -> Vec<String> {
        self.order.lock().unwrap().clone()
    }
    /// Index of the first occurrence of `op` in the call order.
    fn pos(&self, op: &str) -> Option<usize> {
        self.order().iter().position(|o| o == op)
    }
}

/// A configurable stub runtime. `utilization` drives the context-guard branch.
struct StubRuntime {
    rec: Arc<Recorder>,
    utilization: f32,
    evicted: usize,
    post_evict_util: f32,
}

impl StubRuntime {
    fn new(rec: Arc<Recorder>) -> Self {
        Self {
            rec,
            utilization: 0.1,
            evicted: 0,
            post_evict_util: 0.1,
        }
    }
    fn with_utilization(mut self, util: f32, evicted: usize, post: f32) -> Self {
        self.utilization = util;
        self.evicted = evicted;
        self.post_evict_util = post;
        self
    }
}

#[async_trait]
impl OrchestrationRuntime for StubRuntime {
    async fn frontend_instruct(&self, _s: &OrchestrationState) -> anyhow::Result<String> {
        self.rec.mark("frontend_instruct");
        self.rec.instruct_calls.fetch_add(1, Ordering::SeqCst);
        Ok("do the thing".into())
    }
    async fn frontend_compile(&self, s: &OrchestrationState) -> anyhow::Result<String> {
        self.rec.mark("frontend_compile");
        self.rec.compile_calls.fetch_add(1, Ordering::SeqCst);
        Ok(format!(
            "reply: {}",
            s.agent_reply.clone().unwrap_or_default()
        ))
    }
    async fn execute(&self, _s: &OrchestrationState) -> anyhow::Result<ExecuteOutcome> {
        self.rec.mark("execute");
        self.rec.execute_calls.fetch_add(1, Ordering::SeqCst);
        Ok(ExecuteOutcome {
            reply: "canned reasoning reply".into(),
            trace: "step 1\nstep 2\nstep 3".into(),
        })
    }
    async fn compress(&self, _s: &OrchestrationState) -> anyhow::Result<CompressedEntry> {
        self.rec.mark("compress");
        self.rec.compress_calls.fetch_add(1, Ordering::SeqCst);
        Ok(CompressedEntry {
            summary: "compact".into(),
            covered_messages: 3,
        })
    }
    async fn world_diff(&self, s: &OrchestrationState) -> anyhow::Result<WorldDiffEntry> {
        self.rec.mark("world_diff");
        self.rec.world_diff_calls.fetch_add(1, Ordering::SeqCst);
        Ok(WorldDiffEntry {
            seq: s.world_state_diff.entries.len() as u64 + 1,
            note: "mutation".into(),
        })
    }
    async fn context_utilization(&self, _s: &OrchestrationState) -> anyhow::Result<f32> {
        self.rec.mark("context_utilization");
        Ok(self.utilization)
    }
    async fn evict(&self, _s: &OrchestrationState) -> anyhow::Result<EvictionOutcome> {
        self.rec.mark("evict");
        self.rec.evict_calls.fetch_add(1, Ordering::SeqCst);
        Ok(EvictionOutcome {
            evicted: self.evicted,
            new_utilization: self.post_evict_util,
        })
    }
    async fn send_dm(&self, counterpart: &str, body: &str) -> anyhow::Result<()> {
        self.rec.mark("send_dm");
        self.rec
            .dms
            .lock()
            .unwrap()
            .push((counterpart.to_string(), body.to_string()));
        Ok(())
    }
}

fn run(state: OrchestrationState, runtime: StubRuntime) -> OrchestrationState {
    let graph = build_orchestration_graph(Arc::new(runtime), 12, 0.85).expect("graph compiles");
    let exec = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(graph.run(state))
        .expect("graph runs");
    exec.state
}

#[test]
fn full_cycle_walks_all_nodes_and_produces_one_dm_one_compressed_one_diff() {
    let rec = Arc::new(Recorder::default());
    let state = OrchestrationState::seed("h1", "@peer", Vec::new());
    let out = run(state, StubRuntime::new(rec.clone()));

    // Each behaviour-bearing node fired exactly once this cycle.
    assert_eq!(rec.instruct_calls.load(Ordering::SeqCst), 1, "one pass-1");
    assert_eq!(rec.execute_calls.load(Ordering::SeqCst), 1, "one execute");
    assert_eq!(rec.compress_calls.load(Ordering::SeqCst), 1, "one compress");
    assert_eq!(
        rec.world_diff_calls.load(Ordering::SeqCst),
        1,
        "one world_diff"
    );
    assert_eq!(rec.compile_calls.load(Ordering::SeqCst), 1, "one pass-2");

    // Exactly one DM, one compressed-history entry, one world-diff entry.
    assert_eq!(rec.dms.lock().unwrap().len(), 1, "exactly one DM");
    assert_eq!(out.compressed_history.len(), 1, "one compressed row");
    assert_eq!(out.world_state_diff.entries.len(), 1, "one diff entry");
    assert_eq!(out.world_state_diff.entries[0].seq, 1);

    // Terminal state.
    assert_eq!(out.agent_reply.as_deref(), Some("canned reasoning reply"));
    assert_eq!(out.execution_trace, "step 1\nstep 2\nstep 3");
    assert_eq!(
        out.channel_response.as_deref(),
        Some("reply: canned reasoning reply")
    );
    assert!(out.dm_sent);
    assert_eq!(out.pass, 2);
}

#[test]
fn node_order_is_execute_compress_world_diff_then_send_then_guard() {
    let rec = Arc::new(Recorder::default());
    let state = OrchestrationState::seed("h1", "@peer", Vec::new());
    let _ = run(state, StubRuntime::new(rec.clone()));

    // Memory mechanics run in the spec order between execute and the pass-2 reply.
    let execute = rec.pos("execute").expect("execute ran");
    let compress = rec.pos("compress").expect("compress ran");
    let world_diff = rec.pos("world_diff").expect("world_diff ran");
    let compile = rec.pos("frontend_compile").expect("pass-2 ran");
    assert!(execute < compress, "compress runs after execute");
    assert!(compress < world_diff, "world_diff runs after compress");
    assert!(
        world_diff < compile,
        "pass-2 runs after the memory mechanics"
    );

    // Guard-before-END invariant: the context guard runs AFTER the outbound DM
    // (all mutations complete) and is the last op before END.
    let send = rec.pos("send_dm").expect("dm sent");
    let guard = rec.pos("context_utilization").expect("guard ran");
    assert!(
        send < guard,
        "context_guard runs after send_dm (post-mutation)"
    );
    assert_eq!(
        guard,
        rec.order().len() - 1,
        "context_guard is the final op before END: {:?}",
        rec.order()
    );
}

#[test]
fn context_guard_noop_below_threshold_and_evicts_at_or_above() {
    // 0.84 < 0.85 threshold → measure only, no eviction.
    let rec = Arc::new(Recorder::default());
    let out = run(
        OrchestrationState::seed("h1", "@peer", Vec::new()),
        StubRuntime::new(rec.clone()).with_utilization(0.84, 3, 0.2),
    );
    assert_eq!(
        rec.evict_calls.load(Ordering::SeqCst),
        0,
        "no eviction at 0.84"
    );
    assert!((out.context_utilization - 0.84).abs() < f32::EPSILON);
    assert_eq!(out.compressed_history.len(), 1, "compress row retained");

    // 0.86 ≥ 0.85 → evict. The stub reports 1 evicted; state drops that many
    // oldest compressed entries (capped at what exists) and resets utilization.
    let rec = Arc::new(Recorder::default());
    let out = run(
        OrchestrationState::seed("h1", "@peer", Vec::new()),
        StubRuntime::new(rec.clone()).with_utilization(0.86, 1, 0.2),
    );
    assert_eq!(
        rec.evict_calls.load(Ordering::SeqCst),
        1,
        "eviction at 0.86"
    );
    assert!(
        (out.context_utilization - 0.2).abs() < f32::EPSILON,
        "utilization reset"
    );
    // One compressed entry was pushed this cycle and one evicted → empty.
    assert_eq!(
        out.compressed_history.len(),
        0,
        "evicted entry dropped from state"
    );
}

#[test]
fn loop_continuity_adversarial_state_combos_never_cycle_or_double_send() {
    let cases: Vec<(&str, Box<dyn Fn(&mut OrchestrationState)>)> = vec![
        ("cold_start", Box::new(|_s| {})),
        (
            "instructions_without_reply",
            Box::new(|s| s.agent_instructions = Some("stale".into())),
        ),
        (
            "reply_preset",
            Box::new(|s| s.agent_reply = Some("preset".into())),
        ),
        (
            "response_preset",
            Box::new(|s| s.channel_response = Some("already".into())),
        ),
        (
            "reply_and_response_preset",
            Box::new(|s| {
                s.agent_reply = Some("preset".into());
                s.channel_response = Some("already".into());
            }),
        ),
    ];

    for (label, mutate) in cases {
        let rec = Arc::new(Recorder::default());
        let mut state = OrchestrationState::seed("h1", "@peer", Vec::new());
        mutate(&mut state);
        let out = run(state, StubRuntime::new(rec.clone()));

        let dm_count = rec.dms.lock().unwrap().len();
        assert!(
            dm_count <= 1,
            "{label}: sent {dm_count} DMs — must never double-send"
        );
        assert!(
            out.dm_sent,
            "{label}: cycle must reach the terminal send_dm latch"
        );
        assert!(
            out.channel_response.is_some(),
            "{label}: cycle must terminate with a channel_response"
        );
        assert!(
            out.pass <= 12,
            "{label}: {} passes — exceeded backstop",
            out.pass
        );
        if label == "response_preset" || label == "reply_and_response_preset" {
            assert_eq!(
                rec.instruct_calls.load(Ordering::SeqCst),
                0,
                "{label}: pre-set response must not call the front-end LLM"
            );
            assert_eq!(
                dm_count, 1,
                "{label}: still sends the pre-set response once"
            );
        }
    }
}

#[test]
fn topology_is_structurally_valid() {
    let t = orchestration_graph_topology().expect("topology builds");
    assert!(
        t.validation.ok,
        "structural errors: {:?}",
        t.validation.errors
    );
    assert!(!t.nodes.is_empty());
}

#[test]
fn local_master_cycle_skips_the_a2a_frontend_agent() {
    // W2 + master-chat: a local human->OpenHuman cycle (counterpart =
    // LOCAL_MASTER_AGENT) must NOT run the A2A front-end triage/compile — the
    // reasoning core answers directly and its reply is used verbatim.
    let rec = Arc::new(Recorder::default());
    let state = OrchestrationState::seed(
        "master",
        crate::openhuman::orchestration::types::LOCAL_MASTER_AGENT,
        Vec::new(),
    );
    let out = run(state, StubRuntime::new(rec.clone()));

    assert_eq!(
        rec.instruct_calls.load(Ordering::SeqCst),
        0,
        "front-end triage (pass 1) must not run for a local master cycle"
    );
    assert_eq!(
        rec.compile_calls.load(Ordering::SeqCst),
        0,
        "front-end compile (pass 2) must not run for a local master cycle"
    );
    assert!(
        rec.execute_calls.load(Ordering::SeqCst) >= 1,
        "the reasoning core still runs"
    );
    // The core's answer is used verbatim (no "reply: " front-end wrapper).
    assert_eq!(
        out.channel_response.as_deref(),
        Some("canned reasoning reply")
    );
    assert!(out.dm_sent);
}
