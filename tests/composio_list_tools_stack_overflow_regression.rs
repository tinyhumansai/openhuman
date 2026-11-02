//! Regression for the SIGBUS in `crahs.log` (May 17, 2026, build 0.53.49).
//!
//! ## What crashed
//!
//! While a user chat went through
//! `web_channel → orchestrator turn → delegation → sub-agent
//! → composio_list_tools` (the sub-agent runner path below is the one every
//! delegation takes), the
//! in-process core
//! aborted with `EXC_BAD_ACCESS (SIGBUS) — KERN_PROTECTION_FAILURE`
//! at an address inside the **stack guard page** of a `tokio-rt-worker`
//! thread. That's a stack overflow — not a Rust panic. The kernel
//! reported `"Could not determine thread index for stack guard region"`.
//!
//! Bottom of the offending stack:
//!
//! ```text
//! toml_parser::on_array_open / value / document       ← recursive descent
//!   ← toml::de::from_str
//!   ← config::schema::load::parse_config_with_recovery
//!   ← Config::load_or_init_with_env_lookup
//!   ← config::ops::load_config_with_timeout
//!   ← ComposioListToolsTool::execute
//!   ← subagent runner inner loop / run_typed_mode / direct child host
//!   ← SkillDelegationTool::execute   (delegate_to_integrations_agent, since removed)
//!   ← Agent::execute_tool_call / execute_tools / turn
//!   ← web_chat::run_chat_task
//! ```
//!
//! The crash trigger is `composio_list_tools` reloading the config from
//! disk on every call (added in #1710 Wave 4 so a mid-session
//! `composio.mode` toggle is observed). The root cause is the
//! combination of:
//!
//!   1. a ~50-frame async tower from the web channel down into
//!      the sub-agent runner,
//!   2. serde-monomorphised `Visitor` frames for the deeply-nested
//!      [`Config`] struct, each of which can be several KB,
//!   3. the recursive-descent TOML parser piling on a few more frames,
//!   4. all of it running on tokio's default ~2 MB worker-thread stack.
//!
//! ## Why pre-existing e2e tests missed it
//!
//! `tests/json_rpc_e2e.rs` and the WDIO suite invoke `composio_execute`
//! / `composio_list_tools` directly over the RPC transport. That path
//! is only a few frames deep: transport → handler → composio. None of
//! the existing specs string together the full `web channel chat →
//! orchestrator → delegate_to_integrations_agent → sub-agent →
//! composio_list_tools` chain, which is what actually piles the stack
//! high enough to hit the guard page.
//!
//! ## What this test does
//!
//! Faithful reproduction in cargo-test is awkward: we can't easily
//! rebuild the upper chat-channel layers (`web_chat::
//! run_chat_task → Agent::turn → execute_tools → <delegation tool>`)
//! without standing up an HTTP + Socket.IO stack. We drive the production
//! path from direct child execution downward — i.e. everything below a delegation
//! tool's `execute` — on a small tokio worker stack (`SUBAGENT_TOWER_STACK_BYTES`).
//!
//! **Caveat — what this test does and does not catch.** Because the
//! upper ~30 frames are missing, the bare path here fits in 2 MB even
//! without the fix. The bug only manifests when the *full* production
//! tower piles on. So:
//!
//!   * **What this test does catch:** structural regressions in the
//!     sub-agent → composio_list_tools → load_config_with_timeout
//!     dispatch chain — if a refactor breaks that path or makes it
//!     blow even the production-shaped budget, this test trips.
//!   * **What this test does NOT catch:** a re-introduction of the
//!     specific stack-overflow trigger (re-inlining the TOML parse onto
//!     the worker). The production crash needed the full tower above
//!     direct child execution to push 2 MB over.
//!
//! The actual stack-overflow fix is in
//! [`crates/openhuman-core/src/config/schema/load.rs`](`parse_config_with_recovery`)
//! and [`crates/openhuman-core/src/config/ops.rs`](`load_config_with_timeout`):
//!
//!   * `parse_config_with_recovery` runs `toml::from_str::<Config>` on
//!     a blocking-pool thread via `spawn_blocking`. The blocking thread
//!     has a *fresh* stack (no async tower above it), so the parser's
//!     serde-monomorphised `Visitor` frames don't compound with the
//!     agent-harness frames that called it.
//!   * `load_config_with_timeout` is fronted by a process-global cache
//!     keyed on `OPENHUMAN_WORKSPACE`, invalidated by `Config::save()`.
//!     Hot-path consumers (composio per-call reload, #1710 Wave 4) get
//!     a clone, never re-entering the parser.
//!
//! ## Setup
//!
//!   * fresh tokio multi-thread runtime, `thread_stack_size(SUBAGENT_TOWER_STACK_BYTES)`
//!     (3 MiB, well under the 16 MiB production `AGENT_WORKER_STACK_BYTES`), so
//!     frame growth in the tower is caught long before it threatens production,
//!   * `OPENHUMAN_WORKSPACE` pointed at a tempdir with a representative
//!     `config.toml` so the TOML parser does real work,
//!   * direct child execution on a test-owned worker exactly like a delegation tool
//!     does, with a stubbed `ChatModel`
//!     that emits one `composio_list_tools` tool call on iteration 1
//!     and stops on iteration 2.
//!
//! The test also asserts that the real Composio handler ran once and the
//! stub model received its second call. Cargo reports a stack regression as
//! process abort when the tokio worker overflows.

// This stress target is still compiled by normal cargo test. Under llvm-cov,
// coverage instrumentation makes the already-large integration-test binary
// trip CI's linker with SIGBUS before the regression can run.
#![cfg(not(coverage))]

use async_trait::async_trait;
use openhuman_core::agent::subagent_host::ParentExecutionContext;
use openhuman_core::agent::subagent_host::{OpenHumanSubagentHost, SubagentRunOptions};
use openhuman_core::agent_registry::host_definition::AgentDefinitionRegistry;
use openhuman_core::config::AgentConfig;
use openhuman_core::integrations::composio::tools::ComposioListToolsTool;
use parking_lot::Mutex;
use serde_json::json;
use std::sync::Arc;
use tempfile::tempdir;
use tinyagents_definition::{ModelSpec, PromptSource, ToolScope};
use tinyinference_llm::message::AssistantMessage;
use tinyinference_llm::model::{ChatModel, ModelProfile, ModelRequest, ModelResponse};
use tinyinference_llm::tool::ToolCall;
use tinytools::{PermissionLevel, Tool, ToolCategory, ToolResult};
use tinytools_agent::dialect::ToolCallFormat;

/// Worker stack for this guard: 3 MiB, deliberately far below the production
/// `AGENT_WORKER_STACK_BYTES` (16 MiB, which stays the real budget).
///
/// Measured in an unoptimised build (#6379): before the async-frame work the
/// sub-agent tower (direct host lifecycle → child execution → config load →
/// `Config::save`) needed between 3.5 and 4 MiB; it overflowed at 3 MiB. It
/// now peaks near 1.6 MiB (passes at 1664 KiB, overflows at 1536 KiB). 3 MiB
/// leaves ~85% headroom for unrelated growth while still failing on the old
/// frames, so a handler that re-inlines a large future or `Config` copy into
/// this chain trips here long before it could threaten the 16 MiB budget.
const SUBAGENT_TOWER_STACK_BYTES: usize = 3 * 1024 * 1024;

// ── env serialisation (config-rs reads process env) ──────────────────

static ENV_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    let m = ENV_LOCK.get_or_init(|| std::sync::Mutex::new(()));
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, val: &str) -> Self {
        let prev = std::env::var(key).ok();
        // SAFETY: caller holds env_lock().
        unsafe { std::env::set_var(key, val) };
        Self { key, prev }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.prev {
            // SAFETY: caller's env_lock guard is still alive during drop.
            Some(v) => unsafe { std::env::set_var(self.key, v) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

// ── representative on-disk config ────────────────────────────────────
//
// Shape-matches a real user's `config.toml`: many `#[serde(default)]`
// sub-sections so the Visitor traversal during `toml::from_str::<Config>`
// monomorphises through the same code paths as production.

const REPRESENTATIVE_CONFIG_TOML: &str = r#"
schema_version = 2
default_model = "reasoning-v1"
default_temperature = 0.7
temperature_unsupported_models = ["o1*", "o3*", "o4*", "gpt-5*"]
onboarding_completed = true
chat_onboarding_completed = true

[observability]
[autonomy]
[runtime]
[autocomplete]
[reliability]
[scheduler]
[scheduler_gate]
[agent]
[orchestrator]
[composio]
mode = "disabled"
entity_id = "test-entity"
"#;

// ── mock model that emits one composio_list_tools tool call ─────────

struct StubModel {
    iter: Arc<Mutex<usize>>,
    profile: ModelProfile,
}

struct CountedComposioTool {
    inner: ComposioListToolsTool,
    executions: Arc<std::sync::atomic::AtomicUsize>,
    output: Arc<Mutex<Option<String>>>,
}

#[async_trait]
impl Tool for CountedComposioTool {
    fn name(&self) -> &str {
        "composio_list_tools"
    }

    fn description(&self) -> &str {
        "List Composio tools for the stack regression."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.inner.parameters_schema()
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Workflow
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.executions
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let result = self.inner.execute(args).await?;
        *self.output.lock() = Some(result.output().to_string());
        Ok(result)
    }
}

#[async_trait]
impl ChatModel<()> for StubModel {
    fn profile(&self) -> Option<&ModelProfile> {
        Some(&self.profile)
    }

    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        let mut count = self.iter.lock();
        *count += 1;
        if *count == 1 {
            Ok(ModelResponse {
                message: AssistantMessage {
                    id: None,
                    content: Vec::new(),
                    tool_calls: vec![ToolCall::new(
                        "call_1",
                        "composio_list_tools",
                        json!({ "toolkits": ["gmail"] }),
                    )],
                    usage: None,
                    origin: None,
                },
                usage: None,
                finish_reason: Some("tool_calls".into()),
                raw: None,
                resolved_model: None,
                continue_turn: None,
                served_from_cache: false,
                correlation: None,
                resolved_route: None,
            })
        } else {
            Ok(ModelResponse::assistant("done"))
        }
    }
}

// ── the regression itself ────────────────────────────────────────────

/// Structural regression for the path that crashed in `crahs.log`.
/// See module docs for what this test does and does not catch.
///
/// `#[test]` (not `#[tokio::test]`) so the worker stack size is set
/// explicitly. The work runs via `tokio::spawn` so the assertion is
/// performed on a small worker rather than on `block_on`'s driver
/// thread (which inherits the much larger cargo-test main-thread stack
/// and would hide stack-budget regressions).
#[test]
fn composio_list_tools_via_subagent_runs_on_small_worker_stack() {
    // Serialise env mutation across the test binary (other tests may
    // poke OPENHUMAN_WORKSPACE concurrently).
    let _env = env_lock();

    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("config.toml"), REPRESENTATIVE_CONFIG_TOML)
        .expect("write representative config.toml");
    let _ws_guard = EnvGuard::set(
        "OPENHUMAN_WORKSPACE",
        tmp.path().to_str().expect("tempdir path utf-8"),
    );

    // Every runtime that can host an agent turn (the desktop host,
    // `openhuman-core run`, `agent_cli`, embedders) sets
    // `AGENT_WORKER_STACK_BYTES` (16 MiB). The SIGBUS in crahs.log happened on a
    // default ~2 MB worker before that was applied everywhere. This guard runs
    // on a much smaller stack (see `SUBAGENT_TOWER_STACK_BYTES`) so frame growth
    // in the tower is caught early, while production keeps its 16 MiB budget.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_stack_size(SUBAGENT_TOWER_STACK_BYTES)
        .enable_all()
        .build()
        .expect("build runtime");

    // The actual work has to be on a worker thread, not the
    // `block_on` driver thread (which inherits the OS test-runner stack
    // and is much larger). Spawn → join to force the closure onto a
    // small worker.
    rt.block_on(async {
        tokio::spawn(drive_subagent())
            .await
            .expect("subagent task must complete without SIGBUS / panic");
    });
}

async fn drive_subagent() {
    let _ = AgentDefinitionRegistry::init_global_builtins();

    let profile = ModelProfile {
        tool_calling: true,
        ..Default::default()
    };
    let model_calls = Arc::new(Mutex::new(0));
    let model = Arc::new(StubModel {
        iter: model_calls.clone(),
        profile,
    });

    let executions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let tool_output = Arc::new(Mutex::new(None));
    let workspace =
        std::path::PathBuf::from(std::env::var("OPENHUMAN_WORKSPACE").expect("test workspace set"));
    let tool_config = openhuman_core::config::Config {
        config_path: workspace.join("config.toml"),
        workspace_dir: workspace,
        ..Default::default()
    };
    let composio_tool = CountedComposioTool {
        inner: ComposioListToolsTool::new(Arc::new(tool_config)),
        executions: executions.clone(),
        output: tool_output.clone(),
    };
    let composio_spec = Arc::new(composio_tool.spec());
    let parent = ParentExecutionContext {
        agent_definition_id: "orchestrator".into(),
        allowed_subagent_ids: ["stack_regression_worker".to_string()]
            .into_iter()
            .collect(),
        turn_model_source: openhuman_core::agent::tinyagents::TurnModelSource::from_model(model),
        all_tools: Arc::new(vec![Box::new(composio_tool)]),
        all_tool_specs: Arc::new(vec![composio_spec]),
        // #6145: empty means "same surface as `all_tool_specs`" — the
        // catalogue falls back to it, so these stubs keep the behaviour
        // they had before the parent's visible set became its own field.
        visible_tool_specs: Arc::new(Vec::new()),
        visible_tool_names: std::collections::HashSet::new(),
        subagent_tool_ceiling_names: std::collections::HashSet::new(),
        model_name: "test-model".into(),
        temperature: 0.4,
        workspace_dir: std::env::temp_dir(),
        workspace_descriptor: None,
        agent_config: AgentConfig::default(),
        workflows: Arc::new(vec![]),
        memory_context: Arc::new(None),
        session_id: "stack-regression-session".into(),
        channel: "test".into(),
        connected_integrations: vec![],
        tool_call_format: ToolCallFormat::PFormat,
        session_key: "0_stack_regression".into(),
        session_parent_prefix: None,
        on_progress: None,
        run_queue: None,
        host_env: Default::default(),
    };

    let mut def = AgentDefinitionRegistry::global()
        .expect("registry initialised")
        .get("vision_agent")
        .expect("vision worker built-in must exist")
        .clone();
    // Give the worker a test-owned identity and a narrow, neutral surface.
    // Inherit keeps the stub model above on the direct child host path.
    def.id = "stack_regression_worker".into();
    def.display_name = Some("Stack regression worker".into());
    def.when_to_use = "Read the supplied request for this stack regression.".into();
    def.delegate_name = None;
    def.subagents.clear();
    def.max_iterations = 6;
    def.tools = ToolScope::Named(vec!["composio_list_tools".into()]);
    def.model = ModelSpec::Inherit;
    def.system_prompt = PromptSource::Inline(
        "You are a read-only test worker. Use the available tool once, then report its result."
            .into(),
    );
    def.extensions.prompt_builder = None;

    // The worker must execute the real tool handler and return to the stub
    // model. If its thread overflows, the cargo-test process aborts
    // (SIGABRT / SIGBUS) before these assertions can run.
    // The parent is an explicit value on the run options, not ambient state.
    let mut options = SubagentRunOptions::default();
    options.run_context.parent = Some(parent);
    let parent =
        options
            .run_context
            .clone()
            .into_tinyagents(tinyagents_harness::context::RunConfig::new(
                "composio-stack-regression-subagent",
            ));
    let _ = OpenHumanSubagentHost::new(def, options)
        .run(&parent, "list available gmail actions".into())
        .await;
    assert_eq!(
        executions.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the stub model's tool call must execute the real Composio list handler"
    );
    assert_eq!(
        *model_calls.lock(),
        2,
        "the model must continue after the tool result"
    );
    assert!(tool_output
        .lock()
        .as_deref()
        .is_some_and(|output| output.contains("composio is disabled")));
}
