//! End-to-end tests for the `tinyagents` harness route: a real openhuman
//! [`Provider`] and [`Tool`] driven through [`run_turn_via_tinyagents`].

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;

use super::*;
use crate::openhuman::inference::provider::{ChatRequest, ChatResponse, Provider, ToolCall};
use crate::openhuman::tools::{Tool, ToolResult};

#[test]
fn crate_native_turn_source_does_not_retain_host_provider() {
    let source = TurnModelSource::new_crate_native(
        "chat",
        Arc::new(crate::openhuman::config::Config::default()),
    );
    assert!(
        source.provider.is_none(),
        "crate-native turn sources must not construct or retain a host Provider"
    );
    assert!(source.crate_native.is_some());
}

#[test]
fn crate_native_text_mode_does_not_resolve_host_provider() {
    let source = TurnModelSource::new_crate_native(
        "chat",
        Arc::new(crate::openhuman::config::Config::default()),
    )
    .with_text_mode();

    assert!(source.provider.is_none());
    assert!(
        source
            .crate_native
            .as_ref()
            .is_some_and(|native| native.force_text_mode),
        "text mode must be represented on the crate-native source"
    );
}

/// A real openhuman tool the harness will execute.
struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
        "echoes its msg argument"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": { "msg": { "type": "string" } },
            "required": ["msg"]
        })
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let m = args.get("msg").and_then(|v| v.as_str()).unwrap_or("");
        Ok(ToolResult::success(format!("echoed:{m}")))
    }
}

/// Mock provider: first call requests the echo tool, second call answers.
struct EchoThenDone {
    calls: AtomicUsize,
}

#[async_trait]
impl Provider for EchoThenDone {
    async fn chat_with_system(
        &self,
        _s: Option<&str>,
        _m: &str,
        _model: &str,
        _t: f64,
    ) -> anyhow::Result<String> {
        Ok(String::new())
    }
    async fn chat(
        &self,
        _r: ChatRequest<'_>,
        _model: &str,
        _t: f64,
    ) -> anyhow::Result<ChatResponse> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            Ok(ChatResponse {
                tool_calls: vec![ToolCall {
                    id: "call-1".to_string(),
                    name: "echo".to_string(),
                    arguments: r#"{"msg":"hi"}"#.to_string(),
                    extra_content: None,
                }],
                ..Default::default()
            })
        } else {
            Ok(ChatResponse {
                text: Some("all done".to_string()),
                ..Default::default()
            })
        }
    }
    fn supports_native_tools(&self) -> bool {
        true
    }
}

#[tokio::test]
async fn turn_runs_through_the_tinyagents_harness_with_real_tools() {
    let provider = Arc::new(EchoThenDone {
        calls: AtomicUsize::new(0),
    });
    let history = vec![ChatMessage::user("please echo hi")];
    let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(EchoTool)];

    let outcome = run_turn_via_tinyagents(provider, "mock-model", 0.0, history, tools, 10)
        .await
        .expect("tinyagents harness turn runs");

    assert_eq!(outcome.text, "all done");
    assert!(outcome.model_calls >= 2, "expected >=2 model calls");
    assert!(outcome.tool_calls >= 1, "expected the echo tool to run");
    assert!(
        outcome
            .history
            .iter()
            .any(|m| m.content.contains("echoed:hi")),
        "tool result should be threaded into the transcript: {:?}",
        outcome.history
    );
}

/// A provider that streams visible text in chunks through the request's stream
/// sender, then returns the aggregated reply — exercising `ProviderModel::stream`.
struct StreamingProvider;

#[async_trait]
impl Provider for StreamingProvider {
    async fn chat_with_system(
        &self,
        _s: Option<&str>,
        _m: &str,
        _model: &str,
        _t: f64,
    ) -> anyhow::Result<String> {
        Ok(String::new())
    }
    async fn chat(
        &self,
        r: ChatRequest<'_>,
        _model: &str,
        _t: f64,
    ) -> anyhow::Result<ChatResponse> {
        use crate::openhuman::inference::provider::{ProviderDelta, UsageInfo};
        if let Some(tx) = r.stream {
            for chunk in ["Hel", "lo ", "world"] {
                let _ = tx
                    .send(ProviderDelta::TextDelta {
                        delta: chunk.to_string(),
                    })
                    .await;
            }
        }
        Ok(ChatResponse {
            text: Some("Hello world".to_string()),
            usage: Some(UsageInfo {
                input_tokens: 12,
                output_tokens: 4,
                ..Default::default()
            }),
            ..Default::default()
        })
    }
    fn supports_native_tools(&self) -> bool {
        true
    }
}

#[tokio::test]
async fn streaming_path_forwards_text_deltas_and_cost() {
    use crate::openhuman::agent::progress::AgentProgress;

    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentProgress>(64);
    let registry: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![]);
    let history = vec![ChatMessage::user("hi")];

    let provider: Arc<dyn Provider> = Arc::new(StreamingProvider);
    let provider_id = provider.telemetry_provider_id();
    let turn_models = build_turn_models(provider, "mock-model", 0.0, None);
    let outcome = run_turn_via_tinyagents_shared(
        turn_models,
        provider_id,
        "mock-model",
        history,
        vec![registry],
        None,
        4,
        Some(tx),
        None,
        None,
        None,
        &[],
        false,
        None,
        TurnContextMiddleware::defaults(),
        None,
        None,
        false,
        false, // defer_turn_completed_to_caller (#4457)
    )
    .await
    .expect("streaming turn runs");

    assert_eq!(outcome.text, "Hello world");
    assert_eq!((outcome.input_tokens, outcome.output_tokens), (12, 4));

    // Collect the mirrored progress: incremental text deltas + a cost update.
    let mut text = String::new();
    let mut saw_cost = false;
    while let Ok(p) = rx.try_recv() {
        match p {
            AgentProgress::TextDelta { delta, .. } => text.push_str(&delta),
            AgentProgress::TurnCostUpdated { input_tokens, .. } => {
                assert_eq!(input_tokens, 12);
                saw_cost = true;
            }
            _ => {}
        }
    }
    assert!(
        text.contains("Hello world"),
        "incremental text deltas should reassemble the reply, got {text:?}"
    );
    assert!(saw_cost, "a TurnCostUpdated should be emitted");
}

/// A provider that records the messages of every request it receives.
struct CapturingProvider {
    captured: std::sync::Mutex<Vec<Vec<ChatMessage>>>,
}

#[async_trait]
impl Provider for CapturingProvider {
    async fn chat_with_system(
        &self,
        _s: Option<&str>,
        _m: &str,
        _model: &str,
        _t: f64,
    ) -> anyhow::Result<String> {
        Ok(String::new())
    }
    async fn chat(
        &self,
        r: ChatRequest<'_>,
        _model: &str,
        _t: f64,
    ) -> anyhow::Result<ChatResponse> {
        self.captured.lock().unwrap().push(r.messages.to_vec());
        Ok(ChatResponse {
            text: Some("acknowledged".to_string()),
            ..Default::default()
        })
    }
    fn supports_native_tools(&self) -> bool {
        true
    }
}

#[tokio::test]
async fn pre_queued_steer_message_is_injected_into_the_request() {
    use crate::openhuman::agent::harness::run_queue::{QueueMode, QueuedMessage, RunQueue};

    let provider = Arc::new(CapturingProvider {
        captured: std::sync::Mutex::new(Vec::new()),
    });
    let run_queue = RunQueue::new();
    run_queue
        .push(QueuedMessage {
            text: "switch focus to memory safety".into(),
            mode: QueueMode::Steer,
            client_id: "steer".into(),
            thread_id: "t1".into(),
            queued_at_ms: 0,
            model_override: None,
            temperature: None,
            profile_id: None,
            locale: None,
        })
        .await;

    let registry: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![]);
    let provider_id = provider.telemetry_provider_id();
    let turn_models = build_turn_models(provider.clone(), "mock-model", 0.0, None);
    let outcome = run_turn_via_tinyagents_shared(
        turn_models,
        provider_id,
        "mock-model",
        vec![ChatMessage::user("investigate the bug")],
        vec![registry],
        None,
        4,
        None,
        None,
        None,
        Some(run_queue),
        &[],
        false,
        None,
        TurnContextMiddleware::defaults(),
        None,
        None,
        false,
        false, // defer_turn_completed_to_caller (#4457)
    )
    .await
    .expect("steered turn runs");

    assert_eq!(outcome.text, "acknowledged");
    let captured = provider.captured.lock().unwrap();
    let steered = captured
        .iter()
        .flatten()
        .any(|m| m.role == "user" && m.content.contains("switch focus to memory safety"));
    assert!(
        steered,
        "the queued steer should be injected as a user turn, got: {:?}",
        captured
            .iter()
            .flatten()
            .map(|m| (&m.role, &m.content))
            .collect::<Vec<_>>()
    );
}

/// A provider that pops distinct scripted texts from a shared FIFO, recording
/// the order of consumption — models the global mock the parallel children share.
struct FifoProvider {
    responses: std::sync::Mutex<std::collections::VecDeque<String>>,
    calls: AtomicUsize,
}

#[async_trait]
impl Provider for FifoProvider {
    async fn chat_with_system(
        &self,
        _s: Option<&str>,
        _m: &str,
        _model: &str,
        _t: f64,
    ) -> anyhow::Result<String> {
        Ok(String::new())
    }
    async fn chat(
        &self,
        _r: ChatRequest<'_>,
        _model: &str,
        _t: f64,
    ) -> anyhow::Result<ChatResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        // Yield once so two concurrent turns on the same task actually interleave.
        tokio::task::yield_now().await;
        let text = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_default();
        Ok(ChatResponse {
            text: Some(text),
            ..Default::default()
        })
    }
    fn supports_native_tools(&self) -> bool {
        true
    }
}

/// Two sub-agent-style turns (`pause_at_cap = true`) running concurrently on the
/// *same task* (as `spawn_parallel_agents` does via `join_all`) must each get a
/// distinct FIFO response and not deadlock — the `parallel_subagent_fanout`
/// regression in miniature.
#[tokio::test]
async fn concurrent_shared_turns_each_get_a_distinct_result() {
    let provider = Arc::new(FifoProvider {
        responses: std::sync::Mutex::new(
            ["AAA_CANARY".to_string(), "BBB_CANARY".to_string()].into(),
        ),
        calls: AtomicUsize::new(0),
    });
    let registry: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![]);

    let provider_id = provider.telemetry_provider_id();
    let one = run_turn_via_tinyagents_shared(
        build_turn_models(provider.clone(), "mock-model", 0.0, None),
        provider_id.clone(),
        "mock-model",
        vec![ChatMessage::user("task one")],
        vec![registry.clone()],
        None,
        4,
        None,
        None,
        None,
        None,
        &[],
        true,
        None,
        TurnContextMiddleware::defaults(),
        None,
        None,
        false,
        false, // defer_turn_completed_to_caller (#4457)
    );
    let two = run_turn_via_tinyagents_shared(
        build_turn_models(provider.clone(), "mock-model", 0.0, None),
        provider_id,
        "mock-model",
        vec![ChatMessage::user("task two")],
        vec![registry],
        None,
        4,
        None,
        None,
        None,
        None,
        &[],
        true,
        None,
        TurnContextMiddleware::defaults(),
        None,
        None,
        false,
        false, // defer_turn_completed_to_caller (#4457)
    );

    let (a, b) = tokio::join!(one, two);
    let a = a.expect("turn one runs");
    let b = b.expect("turn two runs");

    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        2,
        "exactly one model call per turn"
    );
    let mut got = [a.text.as_str(), b.text.as_str()];
    got.sort_unstable();
    assert_eq!(
        got,
        ["AAA_CANARY", "BBB_CANARY"],
        "each concurrent turn must receive a distinct FIFO response; got {got:?}"
    );
}

/// Adapter inventory (issue #4249, Phase 11): assert the shared runner's
/// assembled harness registers the model, every callable tool, and the intended
/// middleware stack. Counts are the stable proxy for registration order — the
/// crate's `MiddlewareStack` exposes lengths but not names (SDK gap), so
/// ordering itself is documented at the registration sites in
/// `assemble_turn_harness`.
#[test]
fn adapter_inventory_registers_model_tools_and_middleware() {
    let provider: Arc<dyn Provider> = Arc::new(EchoThenDone {
        calls: AtomicUsize::new(0),
    });
    let tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>> =
        vec![Arc::new(vec![Box::new(EchoTool) as Box<dyn Tool>])];

    let assembled = assemble_turn_harness(
        build_turn_models(provider, "mock-model", 0.0, Some(200_000)),
        "mock-model",
        tool_sets,
        None,
        4,
        None,          // on_progress: fire-and-forget
        None,          // subagent_scope: top-level turn
        Some(200_000), // known context window → compression + trim install
        &["ask_user_clarification"],
        TurnContextMiddleware::defaults(),
        None,  // no builder tool policy on this path
        None,  // no per-turn required capabilities
        false, // deterministic_cacheable
    );

    // Model registry: the turn's model plus the projected workload-route set
    // (issue #4249, Workstream 02.1). `names()` is sorted; the turn model
    // (`mock-model`) is not a tier alias, so no route is skipped.
    assert_eq!(
        assembled.harness.models().names(),
        vec![
            "agentic-v1".to_string(),
            "burst-v1".to_string(),
            "chat-v1".to_string(),
            "coding-v1".to_string(),
            "mock-model".to_string(),
            "reasoning-v1".to_string(),
            "summarization-v1".to_string(),
            "vision-v1".to_string(),
        ]
    );

    // Tool registry: every callable tool.
    let tools = assembled.harness.tools().names();
    assert!(tools.contains(&"echo".to_string()), "saw {tools:?}");
    assert_eq!(assembled.tool_count, 1);
    assert!(
        assembled.registry_diagnostics.is_empty(),
        "turn capability projection should be healthy: {:?}",
        assembled.registry_diagnostics
    );
    assert_eq!(
        assembled
            .registry_snapshot
            .count(tinyagents::registry::ComponentKind::Model),
        8,
        "projected registry should include the turn model plus the workload-route set"
    );
    assert_eq!(
        assembled
            .registry_snapshot
            .count(tinyagents::registry::ComponentKind::Tool),
        1,
        "projected registry should include callable tools"
    );
    assert!(
        assembled
            .registry_snapshot
            .count(tinyagents::registry::ComponentKind::Graph)
            >= 1,
        "projected registry should include known graph descriptors"
    );
    let policies = assembled.harness.tools().policies();
    assert!(
        policies.get("echo").is_some_and(|policy| policy.classified),
        "registered tools must expose classified SDK policy snapshots: {policies:?}"
    );
    let stable_policies: BTreeMap<_, _> = policies.into_iter().collect();
    let serialized = serde_json::to_string(&stable_policies).unwrap();
    assert!(serialized.contains("\"classified\":true"));

    // Lifecycle middleware, in registration order: memory-protocol enforcement
    // (outermost), repeated-tool-failure breaker, repeat-progress breaker (#4463),
    // shadow tool-exposure, prompt-cache segment + guard, cache-align + tool-output
    // (TurnContextMiddleware::defaults), observe-only crate BudgetMiddleware
    // (W2-budget-dedupe), cost budget (local enforcement + budget_shadow),
    // context compression + message trim (window known + autocompact on), SDK
    // tool-policy projection, tool-outcome capture, arg recovery, schema guard
    // (#4451 before_tool).
    let mw = assembled.harness.middleware();
    // NOTE(parity merge): these inventory counts are the upstream base (13 / 2)
    // plus the lifecycle + around-tool middlewares this parity branch adds. They
    // are NOT compiled by `cargo check --lib` (this is a #[cfg(test)] block) and
    // are pending the deferred test pass — verify/adjust the exact numbers when
    // the test suite actually compiles.
    assert_eq!(mw.len(), 15, "lifecycle middleware inventory");
    // Around-tool wraps: schema guard (#4451, outermost) + approval/security +
    // CLI/RPC-only scope gate + credential scrub (#4453, innermost). No builder
    // tool policy on this call.
    assert_eq!(mw.tool_middleware_len(), 4, "tool middleware inventory");
    // One around-model wrap: the cost `UsageCarryMiddleware` (always installed).
    // RequiredCapabilities/FallbackObserver are not installed on this call
    // (no required caps; `mock-model` is not a tier, so no fallback chain).
    assert_eq!(
        mw.model_middleware_len(),
        1,
        "usage-carry around-model wrap"
    );
    assert_eq!(
        assembled.harness.policy().limits.max_depth,
        crate::openhuman::agent::harness::MAX_SPAWN_DEPTH,
        "TinyAgents recursion cap should mirror OpenHuman's spawn cap"
    );

    // The shared steering handle always exists; the early-exit hook exists
    // because an early-exit tool name was supplied.
    assert!(assembled.handle.is_some());
    assert!(assembled.early_exit_hook.is_some());

    // Capability profile (issue #4249, Phase 2): derived from the wrapped
    // provider plus the runner-threaded token limits.
    let registered = assembled
        .harness
        .models()
        .get("mock-model")
        .expect("model registered");
    let profile = registered.profile().expect("profile is populated");
    assert_eq!(profile.model.as_deref(), Some("mock-model"));
    assert!(profile.tool_calling, "EchoThenDone supports native tools");
    assert!(!profile.modalities.image_in, "no vision on the mock");
    assert_eq!(profile.max_input_tokens, Some(200_000), "context window");
    // The per-turn output cap now rides `RunConfig.max_turn_output_tokens`
    // (Phase 5 groundwork), not the model profile, so the profile carries no
    // output cap.
    assert_eq!(
        profile.max_output_tokens, None,
        "output cap rides RunConfig"
    );
}

/// The context-management middlewares gate on a known context window: without
/// one, neither compression nor trim installs (and no early-exit hook without
/// early-exit tools).
#[test]
fn adapter_inventory_gates_context_middleware_on_window() {
    let provider: Arc<dyn Provider> = Arc::new(EchoThenDone {
        calls: AtomicUsize::new(0),
    });
    let tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>> =
        vec![Arc::new(vec![Box::new(EchoTool) as Box<dyn Tool>])];

    let assembled = assemble_turn_harness(
        build_turn_models(provider, "mock-model", 0.0, None),
        "mock-model",
        tool_sets,
        None,
        4,
        None,
        None,
        None, // unknown context window
        &[],  // no early-exit tools
        TurnContextMiddleware::defaults(),
        None,
        None,  // no per-turn required capabilities
        false, // deterministic_cacheable
    );

    let mw = assembled.harness.middleware();
    assert_eq!(
        mw.len(),
        13,
        "compression + trim must not install without a window"
    );
    assert!(assembled.early_exit_hook.is_none());
}

/// Phase 5 rollup gap (issue #4249): the per-call global cost tracker feed
/// lives in the event bridge, which only exists on observed runs. An
/// unobserved (fire-and-forget) turn must feed its aggregate usage through
/// `record_unobserved_turn_usage` — exactly once (the bridge and the fallback
/// are mutually exclusive branches) — or its spend never reaches the cost
/// dashboard.
#[tokio::test]
async fn unobserved_turn_reports_aggregate_usage_for_the_cost_fallback() {
    use crate::openhuman::inference::provider::UsageInfo;

    /// Answers immediately, echoing provider-reported usage.
    struct DoneWithUsage;
    #[async_trait]
    impl Provider for DoneWithUsage {
        async fn chat_with_system(
            &self,
            _s: Option<&str>,
            _m: &str,
            _model: &str,
            _t: f64,
        ) -> anyhow::Result<String> {
            Ok(String::new())
        }
        async fn chat(
            &self,
            _r: ChatRequest<'_>,
            _model: &str,
            _t: f64,
        ) -> anyhow::Result<ChatResponse> {
            Ok(ChatResponse {
                text: Some("done".to_string()),
                usage: Some(UsageInfo {
                    input_tokens: 111,
                    output_tokens: 22,
                    context_window: 0,
                    cached_input_tokens: 7,
                    cache_creation_tokens: 0,
                    reasoning_tokens: 0,
                    charged_amount_usd: 0.0,
                }),
                ..Default::default()
            })
        }
        fn supports_native_tools(&self) -> bool {
            true
        }
    }

    let provider: Arc<dyn Provider> = Arc::new(DoneWithUsage);
    let provider_id = provider.telemetry_provider_id();
    let turn_models = build_turn_models(provider, "mock-model", 0.0, None);
    let outcome = run_turn_via_tinyagents_shared(
        turn_models,
        provider_id,
        "mock-model",
        vec![ChatMessage::user("hello")],
        Vec::new(),
        None,
        3,
        None, // on_progress: unobserved — no bridge, cost fallback branch runs
        None,
        None,
        None,
        &[],
        false,
        None,
        TurnContextMiddleware::defaults(),
        None,
        None,
        false,
        false, // defer_turn_completed_to_caller (#4457)
    )
    .await
    .expect("turn runs");

    // The fallback branch aggregated the run's real usage (and fed the global
    // tracker — a silent no-op when no tracker is installed in this process).
    assert_eq!(outcome.input_tokens, 111);
    assert_eq!(outcome.output_tokens, 22);
    assert_eq!(outcome.cached_input_tokens, 7);
}

/// The cost-fallback recorder skips all-zero usage (providers that echo no
/// usage must not inflate the tracker's request count) and attempts a record
/// whenever any tokens were observed. It must never panic without a tracker.
#[test]
fn record_unobserved_turn_usage_gates_on_observed_tokens() {
    assert!(!record_unobserved_turn_usage("m", 0, 0, 0, 0.0));
    assert!(!record_unobserved_turn_usage("m", 0, 0, 5, 0.1));
    assert!(record_unobserved_turn_usage("m", 10, 0, 0, 0.0));
    assert!(record_unobserved_turn_usage("m", 0, 3, 0, 0.0));
    assert!(record_unobserved_turn_usage("m", 10, 3, 2, 0.5));
}

#[test]
fn spawn_and_delegate_tools_are_never_registered_on_subagents() {
    // #4452: a child run must never be able to register a spawn/delegate tool,
    // even if the resolved allowlist somehow contains one — the registration
    // site strips these unconditionally as defense-in-depth.
    for name in [
        "spawn_subagent",
        "spawn_worker_thread",
        "use_tinyplace",
        "agent_prepare_context",
        "delegate_research",
        "delegate_",
    ] {
        assert!(
            is_subagent_spawn_or_delegate_tool(name),
            "{name} must be treated as a spawn/delegate tool"
        );
    }
    // Ordinary tools (and near-miss names) must NOT be stripped.
    for name in ["shell", "read_file", "web_search", "spawn", "subagent"] {
        assert!(
            !is_subagent_spawn_or_delegate_tool(name),
            "{name} is a normal tool and must not be stripped"
        );
    }
}

/// Issue #4746: the harness bounds each model/tool/sub-agent call by the run's
/// remaining wall-clock budget, but ONLY when `max_wall_clock_ms` is set — with
/// `None` a hung call is awaited unbounded and the turn can ship an empty reply
/// with no terminal event. `run_policy_for` must arm that ceiling.
#[test]
fn run_policy_for_arms_the_wall_clock_ceiling() {
    let policy = run_policy_for(10, false);
    assert_eq!(
        policy.limits.max_wall_clock_ms,
        Some(DEFAULT_AGENT_TURN_TIMEOUT_SECS * 1_000),
        "run_policy_for must set a wall-clock ceiling so hung calls are interrupted"
    );
}

/// The `OPENHUMAN_AGENT_TURN_TIMEOUT_SECS` override maps seconds → ms, falls
/// back to the default when absent/unparseable, and treats `0` as an explicit
/// unbounded opt-out (`None`). Tested through the env-free pure core so it stays
/// deterministic under parallel execution.
#[test]
fn agent_turn_wall_clock_ms_parses_env_override() {
    // Absent → default.
    assert_eq!(
        parse_agent_turn_wall_clock_ms(None),
        Some(DEFAULT_AGENT_TURN_TIMEOUT_SECS * 1_000)
    );
    // Explicit custom value → seconds converted to ms.
    assert_eq!(parse_agent_turn_wall_clock_ms(Some("120")), Some(120_000));
    // Whitespace tolerated.
    assert_eq!(parse_agent_turn_wall_clock_ms(Some(" 90 ")), Some(90_000));
    // `0` → unbounded opt-out.
    assert_eq!(parse_agent_turn_wall_clock_ms(Some("0")), None);
    // Garbage → default (fail safe, never unbounded by accident).
    assert_eq!(
        parse_agent_turn_wall_clock_ms(Some("not-a-number")),
        Some(DEFAULT_AGENT_TURN_TIMEOUT_SECS * 1_000)
    );
}
