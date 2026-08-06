//! Deterministic offline `ChatModel` mocks installed via
//! `test_provider_override` (honoured only under the `rss-bench` feature).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use openhuman_core::openhuman::inference::provider::types::{ChatResponse, ToolCall};
use tinyagents::harness::message::{AssistantMessage, ContentBlock, Message};
use tinyagents::harness::model::{ChatModel, ModelRequest, ModelResponse};
use tinyagents::harness::tool::ToolCall as TinyAgentsToolCall;

/// A plain `ChatResponse` carrying only text (no tool calls).
pub fn response(text: &str) -> ChatResponse {
    ChatResponse {
        text: Some(text.into()),
        tool_calls: Vec::new(),
        usage: None,
        reasoning_content: None,
    }
}

fn joined_request(request: &ModelRequest) -> String {
    request
        .messages
        .iter()
        .map(|message| {
            let role = match message {
                Message::System(_) => "system",
                Message::User(_) => "user",
                Message::Assistant(_) => "assistant",
                Message::Tool(_) => "tool",
            };
            format!("{role}: {}", message.text())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn model_response(response: ChatResponse) -> ModelResponse {
    let content = response
        .text
        .filter(|text| !text.is_empty())
        .map(|text| vec![ContentBlock::Text(text)])
        .unwrap_or_default();
    let tool_calls = response
        .tool_calls
        .into_iter()
        .map(|call| {
            TinyAgentsToolCall::new(
                call.id,
                call.name,
                serde_json::from_str(&call.arguments).unwrap_or(serde_json::Value::Null),
            )
        })
        .collect::<Vec<_>>();
    ModelResponse {
        finish_reason: (!tool_calls.is_empty()).then(|| "tool_calls".to_string()),
        message: AssistantMessage {
            id: None,
            content,
            tool_calls,
            usage: None,
        },
        usage: None,
        raw: None,
        resolved_model: None,
        continue_turn: None,
    }
}

/// Records every prompt it sees so scenarios can assert what ran.
fn record(prompts: &Mutex<Vec<String>>, joined: &str) {
    prompts
        .lock()
        .expect("mock prompt lock")
        .push(joined.into());
}

/// Read `key` as a `u64`, falling back to `default` when unset/unparsable.
fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default)
}

/// Stable per-researcher marker embedded in a delegated subagent's prompt.
/// Zero-padded so `..._001` is never a substring of `..._011` — the storm mock
/// routes by exact marker containment across K up to 32.
pub fn subagent_marker(index: usize) -> String {
    format!("LIB_PROFILE_SUBAGENT_{index:03}")
}

/// The finding text a delegated researcher returns for `index`. The merge turn
/// is detected by all K of these being present in the conversation.
pub fn finding_text(index: usize) -> String {
    format!("Finding {index:03}: researcher {index} reports healthy.")
}

/// Text the orchestrator returns once it has merged every researcher finding;
/// its arrival in the parent (subconscious) conversation ends the storm turn.
pub const MERGE_SENTINEL: &str = "STORM_MERGE_COMPLETE";

/// Shared, dependency-free latency sampler driven by the standard env knobs
/// (`OPENHUMAN_PROFILE_MOCK_LATENCY_MS` mean, `OPENHUMAN_PROFILE_MOCK_JITTER_MS`
/// jitter, default `mean / 4`). Reused by both [`LatencyMock`] and
/// [`SubagentMock`] so a delegated subagent turn can carry realistic latency.
pub struct LatencyKnobs {
    mean_ms: u64,
    jitter_ms: u64,
    counter: AtomicU64,
}

impl LatencyKnobs {
    pub fn from_env() -> Self {
        let mean_ms = env_u64("OPENHUMAN_PROFILE_MOCK_LATENCY_MS", 0);
        let jitter_ms = env_u64("OPENHUMAN_PROFILE_MOCK_JITTER_MS", mean_ms / 4);
        eprintln!("[library-profile] LatencyKnobs mean_ms={mean_ms} jitter_ms={jitter_ms}");
        Self {
            mean_ms,
            jitter_ms,
            counter: AtomicU64::new(0),
        }
    }

    /// Sample `mean ± jitter` (clamped at zero) via a seeded xorshift step. The
    /// seed advances per call so successive turns get distinct latencies.
    pub fn sample_ms(&self) -> u64 {
        if self.mean_ms == 0 && self.jitter_ms == 0 {
            return 0;
        }
        let seed = self.counter.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        // xorshift64 — deterministic, dependency-free pseudo-randomness.
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        let span = self.jitter_ms.saturating_mul(2).saturating_add(1);
        let delta = (x % span) as i64 - self.jitter_ms as i64;
        (self.mean_ms as i64 + delta).max(0) as u64
    }

    /// Sleep a sampled latency and return the ms slept (0 when disabled).
    pub async fn sleep_sampled(&self) -> u64 {
        let ms = self.sample_ms();
        if ms > 0 {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }
        ms
    }
}

/// Orchestration mock: the first (orchestrator) turn emits a
/// `spawn_parallel_agents` tool call fanning out to **K** researchers; each
/// researcher turn returns its finding; the final merge turn returns plain
/// text once all K findings are present.
///
/// `width` = K parallel researchers. `new()` keeps the original two-researcher
/// shape (K = 2, no injected latency, driven from the subconscious which has no
/// `spawn_parallel_agents` — the tool call is rejected and its markers echo
/// back, which is all the `subagents` scenario asserts). `with_width(k)` drives
/// the **orchestrator directly** and scripts the full delegation chain, so it
/// must first hand off via `delegate_orchestrator`-free direct fan-out — this is
/// the `subagent-storm` fuzz-width path.
pub struct SubagentMock {
    pub prompts: Mutex<Vec<String>>,
    /// Actual wall-time (ms) of each *researcher* chat call, for percentiles.
    pub researcher_latencies_ms: Mutex<Vec<u128>>,
    width: usize,
    latency: LatencyKnobs,
    /// `true` for `with_width` (orchestrator-driven storm): route the full
    /// agent-aware chain. `false` for `new` (subconscious-driven `subagents`):
    /// the original simple routing that emits the fan-out directly.
    orchestrator_driven: bool,
    /// Increments per fan-out so successive `spawn_parallel_agents` calls carry
    /// distinct task prompts.
    spawn_nonce: AtomicU64,
}

impl SubagentMock {
    /// Two researchers, no injected latency (the original `subagents` shape).
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            prompts: Mutex::new(Vec::new()),
            researcher_latencies_ms: Mutex::new(Vec::new()),
            width: 2,
            latency: LatencyKnobs {
                mean_ms: 0,
                jitter_ms: 0,
                counter: AtomicU64::new(0),
            },
            orchestrator_driven: false,
            spawn_nonce: AtomicU64::new(0),
        })
    }

    /// K researchers with per-researcher latency drawn from the env knobs, driven
    /// directly against the orchestrator agent (full agent-aware chain).
    pub fn with_width(width: usize) -> Arc<Self> {
        Arc::new(Self {
            prompts: Mutex::new(Vec::new()),
            researcher_latencies_ms: Mutex::new(Vec::new()),
            width: width.max(1),
            latency: LatencyKnobs::from_env(),
            orchestrator_driven: true,
            spawn_nonce: AtomicU64::new(0),
        })
    }

    /// Which researcher (1-based) this prompt is for, if any. A researcher
    /// prompt embeds exactly one `subagent_marker`.
    fn researcher_index(&self, joined: &str) -> Option<usize> {
        (1..=self.width).find(|&i| joined.contains(&subagent_marker(i)))
    }

    /// True once every researcher's finding is present — the merge turn.
    fn is_merge(&self, joined: &str) -> bool {
        (1..=self.width).all(|i| joined.contains(&finding_text(i)))
    }

    /// True when this call is a real researcher worker turn: exactly one task
    /// marker is present and the executing agent is neither the orchestrator nor
    /// the subconscious (their Tool Policy Boundary headers name them, and their
    /// merge/echo turns also carry every marker). The researcher agent's system
    /// prompt names it `Researcher`, so it matches neither header string.
    fn is_researcher_turn(&self, joined: &str) -> bool {
        self.researcher_index(joined).is_some()
            && !joined.contains("Agent: orchestrator")
            && !joined.contains("Agent: subconscious")
    }

    /// Build the fan-out tool call delegating to K parallel researchers. Only
    /// valid on an **orchestrator** turn — the subconscious has no
    /// `spawn_parallel_agents` tool, so we `delegate_orchestrator` there first.
    fn spawn_call(&self) -> ChatResponse {
        let nonce = self.spawn_nonce.fetch_add(1, Ordering::Relaxed);
        let tasks: Vec<serde_json::Value> = (1..=self.width)
            .map(|i| {
                serde_json::json!({
                    "agent_id": "researcher",
                    // The nonce keeps each fan-out's tasks byte-distinct so the
                    // parallel-graph result cache can't short-circuit a re-spawn.
                    "prompt": format!("{} [spawn {nonce}]: inspect subsystem {i}", subagent_marker(i)),
                    "ownership": format!("scope: subsystem-{i}-spawn-{nonce}")
                })
            })
            .collect();
        ChatResponse {
            text: Some(format!("Delegating to {} researchers.", self.width)),
            tool_calls: vec![ToolCall {
                id: "profile-parallel-call".into(),
                name: "spawn_parallel_agents".into(),
                arguments: serde_json::json!({ "tasks": tasks }).to_string(),
                extra_content: None,
            }],
            usage: None,
            reasoning_content: None,
        }
    }

    /// The subconscious's first turn: hand the task to the orchestrator (which
    /// owns `spawn_parallel_agents` and allows the `researcher` subagent).
    fn delegate_orchestrator_call(&self) -> ChatResponse {
        ChatResponse {
            text: Some("Delegating to the orchestrator for a parallel research fan-out.".into()),
            tool_calls: vec![ToolCall {
                id: "profile-delegate-orchestrator".into(),
                name: "delegate_orchestrator".into(),
                arguments: serde_json::json!({
                    "prompt": "Research every subsystem in parallel and merge the findings."
                })
                .to_string(),
                extra_content: None,
            }],
            usage: None,
            reasoning_content: None,
        }
    }

    /// Classify the turn by the *executing agent* (from the Tool Policy Boundary
    /// header) and script the real delegation chain:
    /// subconscious → `delegate_orchestrator` → orchestrator →
    /// `spawn_parallel_agents(K)` → K researchers → orchestrator merge →
    /// subconscious final. No latency/recording here — the async `chat` wrappers
    /// handle sleeping + latency capture around this.
    fn reply(&self, joined: &str) -> ChatResponse {
        if self.orchestrator_driven {
            return self.reply_orchestrator_driven(joined);
        }
        // Original `subagents` routing (subconscious-driven): merge once both
        // findings are present, answer a researcher's marker with its finding,
        // else emit the fan-out directly. The subconscious rejects the unknown
        // `spawn_parallel_agents`, echoing the markers back — which is all the
        // `subagents` scenario asserts.
        if self.is_merge(joined) {
            return response("Merged all researcher findings.");
        }
        if let Some(i) = self.researcher_index(joined) {
            return response(&finding_text(i));
        }
        self.spawn_call()
    }

    /// Agent-aware routing for the orchestrator-driven storm.
    fn reply_orchestrator_driven(&self, joined: &str) -> ChatResponse {
        // Researcher worker: return its finding.
        if self.is_researcher_turn(joined) {
            let i = self
                .researcher_index(joined)
                .expect("researcher turn has a marker");
            return response(&finding_text(i));
        }
        // Orchestrator: fan out, then merge once every finding is back.
        if joined.contains("Agent: orchestrator") {
            if self.is_merge(joined) {
                return response(MERGE_SENTINEL);
            }
            return self.spawn_call();
        }
        // Parent (subconscious / any other): finish once the orchestrator's
        // merged result has flowed back; otherwise delegate to the orchestrator.
        if joined.contains(MERGE_SENTINEL) {
            return response("Storm complete: merged every researcher's finding.");
        }
        self.delegate_orchestrator_call()
    }
}

#[async_trait]
impl ChatModel<()> for SubagentMock {
    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyagents::Result<ModelResponse> {
        Ok(model_response(
            self.dispatch(&joined_request(&request)).await,
        ))
    }
}

impl SubagentMock {
    /// Record the prompt, sleep a sampled latency for *researcher* calls (and
    /// capture their wall time), then return the classified response.
    async fn dispatch(&self, joined: &str) -> ChatResponse {
        record(&self.prompts, joined);
        let is_researcher = self.is_researcher_turn(joined);
        let started = std::time::Instant::now();
        if is_researcher {
            self.latency.sleep_sampled().await;
        }
        let resp = self.reply(joined);
        if is_researcher {
            self.researcher_latencies_ms
                .lock()
                .expect("mock latency lock")
                .push(started.elapsed().as_millis());
        }
        resp
    }
}

/// Latency-configurable text-only mock used by the `fleet` scenario. Before
/// returning its fixed answer it sleeps a sampled latency: a mean from
/// `OPENHUMAN_PROFILE_MOCK_LATENCY_MS` (default `0` = no sleep) with jitter
/// `± OPENHUMAN_PROFILE_MOCK_JITTER_MS` (default `mean / 4`). Per-call jitter is
/// derived from a seeded xorshift counter — deterministic and dependency-free
/// (no `rand` crate).
pub struct LatencyMock {
    text: String,
    latency: LatencyKnobs,
    pub prompts: Mutex<Vec<String>>,
}

impl LatencyMock {
    /// Build from the standard env knobs.
    pub fn from_env(text: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            text: text.into(),
            latency: LatencyKnobs::from_env(),
            prompts: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl ChatModel<()> for LatencyMock {
    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyagents::Result<ModelResponse> {
        self.latency.sleep_sampled().await;
        let joined = joined_request(&request);
        record(&self.prompts, &joined);
        Ok(model_response(response(&self.text)))
    }
}

/// Text-only mock: always returns a fixed direct answer, never a tool call.
/// Used by the single-turn / workflow scenarios that must NOT delegate.
pub struct PlainTextMock {
    text: String,
    pub prompts: Mutex<Vec<String>>,
}

impl PlainTextMock {
    pub fn new(text: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            text: text.into(),
            prompts: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl ChatModel<()> for PlainTextMock {
    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyagents::Result<ModelResponse> {
        let joined = joined_request(&request);
        record(&self.prompts, &joined);
        Ok(model_response(response(&self.text)))
    }
}

/// The stdout marker the profiling `node` step prints. Its presence in the run
/// conversation proves the real interpreter child executed (not merely that a
/// tool call was emitted).
pub const NODE_MARKER: &str = "PROFILE_NODE_RESULT";

/// Skill-run mock: the first turn emits a `node_exec` tool call running a short,
/// real JavaScript computation (which spawns a genuine `node` child process);
/// once its printed JSON (carrying [`NODE_MARKER`]) rides back into the
/// conversation, the mock returns a plain final answer so the agent turn
/// completes. This scripts exactly the tool call the code-executor specialist
/// needs to spawn the language runtime whose cost we measure.
pub struct SkillRunMock {
    code: String,
    node_call_emitted: AtomicU64,
    node_output_seen: AtomicU64,
    pub prompts: Mutex<Vec<String>>,
}

impl SkillRunMock {
    pub fn new() -> Arc<Self> {
        // A real computation, a live allocation, and a ~1.2s busy-wait so the
        // child stays resident long enough for the tree sampler (15 ms poll) to
        // catch it. The JSON it prints carries NODE_MARKER.
        let code = format!(
            "const start = Date.now();\n\
             const buf = [];\n\
             let sum = 0;\n\
             for (let i = 0; i < 500000; i++) {{ buf.push(i % 97); sum += i; }}\n\
             while (Date.now() - start < 1200) {{ sum += buf.length; }}\n\
             console.log(JSON.stringify({{ marker: '{NODE_MARKER}', sum, kept: buf.length }}));\n"
        );
        Arc::new(Self {
            code,
            node_call_emitted: AtomicU64::new(0),
            node_output_seen: AtomicU64::new(0),
            prompts: Mutex::new(Vec::new()),
        })
    }

    /// True once the `node_exec` tool call has been emitted.
    pub fn node_call_emitted(&self) -> bool {
        self.node_call_emitted.load(Ordering::Relaxed) > 0
    }

    /// True once the node child's printed output flowed back into the turn —
    /// i.e. the interpreter child actually ran and printed.
    pub fn node_output_seen(&self) -> bool {
        self.node_output_seen.load(Ordering::Relaxed) > 0
    }

    fn reply(&self, joined: &str) -> ChatResponse {
        record(&self.prompts, joined);
        if joined.contains(NODE_MARKER) {
            self.node_output_seen.store(1, Ordering::Relaxed);
            return response(
                "Skill complete: the Node.js step computed the value and it checks out.",
            );
        }
        self.node_call_emitted.store(1, Ordering::Relaxed);
        ChatResponse {
            text: Some("Running the JavaScript computation step.".into()),
            tool_calls: vec![ToolCall {
                id: "profile-node-call".into(),
                name: "node_exec".into(),
                arguments: serde_json::json!({ "inline_code": self.code }).to_string(),
                extra_content: None,
            }],
            usage: None,
            reasoning_content: None,
        }
    }
}

#[async_trait]
impl ChatModel<()> for SkillRunMock {
    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyagents::Result<ModelResponse> {
        Ok(model_response(self.reply(&joined_request(&request))))
    }
}
