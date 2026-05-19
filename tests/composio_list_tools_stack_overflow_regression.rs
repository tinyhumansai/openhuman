//! Regression for the SIGBUS in `crahs.log` (May 17, 2026, build 0.53.49).
//!
//! ## What crashed
//!
//! While a user chat went through
//! `web_channel → orchestrator turn → delegate_to_integrations_agent
//! → integrations_agent → composio_list_tools`, the in-process core
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
//!   ← subagent_runner::run_inner_loop / run_typed_mode / run_subagent
//!   ← SkillDelegationTool::execute   (delegate_to_integrations_agent)
//!   ← Agent::execute_tool_call / execute_tools / turn
//!   ← channels::providers::web::run_chat_task
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
//! rebuild the upper chat-channel layers (`channels::providers::web::
//! run_chat_task → Agent::turn → execute_tools → SkillDelegationTool`)
//! without standing up an HTTP + Socket.IO stack. We drive the production
//! path from `run_subagent` downward — i.e. everything below
//! `delegate_to_integrations_agent::execute` — on a production-realistic
//! 2 MB tokio worker stack.
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
//!     `run_subagent` to push 2 MB over.
//!
//! The actual stack-overflow fix is in
//! [`src/openhuman/config/schema/load.rs`](`parse_config_with_recovery`)
//! and [`src/openhuman/config/ops.rs`](`load_config_with_timeout`):
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
//!   * fresh tokio multi-thread runtime, `thread_stack_size(2 << 20)`
//!     (production default), so the test runs in the same stack budget
//!     production does — anything larger would let dormant regressions
//!     hide for longer,
//!   * `OPENHUMAN_WORKSPACE` pointed at a tempdir with a representative
//!     `config.toml` so the TOML parser does real work,
//!   * `run_subagent(integrations_agent)` exactly like
//!     `delegate_to_integrations_agent` does, with a stubbed `Provider`
//!     that emits one `composio_list_tools` tool call on iteration 1
//!     and stops on iteration 2.
//!
//! Assertion is implicit: cargo reports the test as failed when the
//! tokio runtime aborts with stack overflow.

use anyhow::Result;
use async_trait::async_trait;
use openhuman_core::openhuman::agent::harness::definition::{AgentDefinitionRegistry, ModelSpec};
use openhuman_core::openhuman::agent::harness::{
    run_subagent, with_parent_context, ParentExecutionContext, SubagentRunOptions,
};
use openhuman_core::openhuman::config::AgentConfig;
use openhuman_core::openhuman::context::prompt::ToolCallFormat;
use openhuman_core::openhuman::inference::provider::{
    ChatRequest, ChatResponse, Provider, ToolCall,
};
use openhuman_core::openhuman::memory::{
    Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts,
};
use parking_lot::Mutex;
use serde_json::json;
use std::sync::Arc;
use tempfile::tempdir;

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
[screen_intelligence]
[autocomplete]
[reliability]
[scheduler]
[scheduler_gate]
[agent]
[orchestrator]
[composio]
mode = "backend"
entity_id = "test-entity"
"#;

// ── mock provider that emits one composio_list_tools tool call ──────

struct StubProvider {
    iter: Arc<Mutex<usize>>,
}

#[async_trait]
impl Provider for StubProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> Result<String> {
        Ok("ok".into())
    }

    async fn chat(
        &self,
        _request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> Result<ChatResponse> {
        let mut count = self.iter.lock();
        *count += 1;
        if *count == 1 {
            Ok(ChatResponse {
                text: Some("listing available gmail actions".into()),
                tool_calls: vec![ToolCall {
                    id: "call_1".into(),
                    name: "composio_list_tools".into(),
                    arguments: json!({ "toolkits": ["gmail"] }).to_string(),
                }],
                usage: None,
            })
        } else {
            Ok(ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
            })
        }
    }

    fn supports_native_tools(&self) -> bool {
        true
    }
}

// ── stub memory ──────────────────────────────────────────────────────

struct StubMemory;

#[async_trait]
impl Memory for StubMemory {
    async fn store(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: MemoryCategory,
        _: Option<&str>,
    ) -> Result<()> {
        Ok(())
    }
    async fn recall(&self, _: &str, _: usize, _: RecallOpts<'_>) -> Result<Vec<MemoryEntry>> {
        Ok(vec![])
    }
    async fn get(&self, _: &str, _: &str) -> Result<Option<MemoryEntry>> {
        Ok(None)
    }
    async fn list(
        &self,
        _: Option<&str>,
        _: Option<&MemoryCategory>,
        _: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(vec![])
    }
    async fn forget(&self, _: &str, _: &str) -> Result<bool> {
        Ok(true)
    }
    async fn namespace_summaries(&self) -> Result<Vec<NamespaceSummary>> {
        Ok(vec![])
    }
    async fn count(&self) -> Result<usize> {
        Ok(0)
    }
    async fn health_check(&self) -> bool {
        true
    }
    fn name(&self) -> &str {
        "stub"
    }
}

// ── the regression itself ────────────────────────────────────────────

/// Structural regression for the path that crashed in `crahs.log`.
/// See module docs for what this test does and does not catch.
///
/// `#[test]` (not `#[tokio::test]`) so the worker stack size is set
/// explicitly. The work runs via `tokio::spawn` so the assertion is
/// performed on a 2 MB worker rather than on `block_on`'s driver
/// thread (which inherits the much larger cargo-test main-thread stack
/// and would hide stack-budget regressions).
#[test]
fn composio_list_tools_via_subagent_runs_on_production_worker_stack() {
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

    // Production tokio worker stack default is ~2 MB. The SIGBUS in
    // crahs.log occurred at an address inside a 2080 KB stack region
    // (`Stack 302648000-302850000`). Reproduce that budget exactly.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_stack_size(2 * 1024 * 1024)
        .enable_all()
        .build()
        .expect("build runtime");

    // The actual work has to be on a worker thread, not the
    // `block_on` driver thread (which inherits the OS test-runner stack
    // and is much larger). Spawn → join to force the closure onto a
    // 2 MB worker.
    rt.block_on(async {
        tokio::spawn(drive_subagent())
            .await
            .expect("subagent task must complete without SIGBUS / panic");
    });
}

async fn drive_subagent() {
    let _ = AgentDefinitionRegistry::init_global_builtins();

    let provider = Arc::new(StubProvider {
        iter: Arc::new(Mutex::new(0)),
    });

    let parent = ParentExecutionContext {
        provider: provider.clone(),
        all_tools: Arc::new(vec![]),
        all_tool_specs: Arc::new(vec![]),
        model_name: "test-model".into(),
        temperature: 0.4,
        workspace_dir: std::env::temp_dir(),
        memory: Arc::new(StubMemory),
        agent_config: AgentConfig::default(),
        skills: Arc::new(vec![]),
        memory_context: Arc::new(None),
        session_id: "stack-regression-session".into(),
        channel: "test".into(),
        connected_integrations: vec![],
        tool_call_format: ToolCallFormat::PFormat,
        session_key: "0_stack_regression".into(),
        session_parent_prefix: None,
        on_progress: None,
    };

    let mut def = AgentDefinitionRegistry::global()
        .expect("registry initialised")
        .get("integrations_agent")
        .expect("integrations_agent built-in must exist")
        .clone();
    // The shipped `integrations_agent` definition has `model.hint =
    // "agentic"`, which would otherwise build a fresh provider via the
    // workload factory and try to hit the real backend. Override to
    // Inherit so the stub provider above receives the request — same
    // trick used in `tests/calendar_grounding_e2e.rs`.
    def.model = ModelSpec::Inherit;

    // The assertion is implicit: if the worker thread overflows its
    // stack the cargo-test process aborts (SIGABRT / SIGBUS) and no
    // test report is emitted. We don't care whether the sub-agent
    // semantically "succeeded" — only that the runtime did not abort.
    let _ = with_parent_context(parent, async move {
        run_subagent(
            &def,
            "list available gmail actions",
            SubagentRunOptions::default(),
        )
        .await
    })
    .await;
}
