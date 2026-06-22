//! Fully hermetic FULL-STACK e2e: the REAL gate + REAL long-lived session
//! run against a **mocked LLM provider** — no network, no Ollama, no real
//! model anywhere (cloud and local provider funnels are both overridden).
//!
//! Unlike `subconscious_conversation_e2e.rs` (which injects scripted Gate /
//! SessionExecutor *above* the model), this test mocks at the *provider*
//! layer, so the production code paths run for real:
//!   - `GatePass::evaluate` → `agent::triage::run_triage` → real provider
//!     funnel → mock → real triage parse → real promote/drop mapping.
//!   - `LongLivedSession::process_promoted` → `Agent::from_config` (the real
//!     orchestrator agent + tool loop) → mock → real reserved-thread persistence.
//!
//! The mock is installed via the factory's `test_provider_override` seam,
//! which both provider funnels consult first.
//!
//! Gated on the off-by-default `e2e-test-support` feature (the seam is only
//! compiled then). Run with:
//! `cargo test --features e2e-test-support --test subconscious_fullstack_e2e -- --nocapture`
#![cfg(feature = "e2e-test-support")]

use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use async_trait::async_trait;

use openhuman_core::core::event_bus::{init_global, DomainEvent};
use openhuman_core::openhuman::agent::harness::AgentDefinitionRegistry;
use openhuman_core::openhuman::config::schema::SubconsciousMode;
use openhuman_core::openhuman::inference::provider::factory::test_provider_override;
use openhuman_core::openhuman::inference::provider::traits::{
    ChatRequest, ChatResponse, ProviderCapabilities, ToolCall,
};
use openhuman_core::openhuman::inference::provider::Provider;
use openhuman_core::openhuman::subconscious::LongLivedSession;
use openhuman_core::openhuman::subconscious_triggers::{normalize, GatePass};

// ─────────────────────────────────────────────────────────────────────────────
// Mock LLM provider — deterministic, content-routed, no network.
// ─────────────────────────────────────────────────────────────────────────────

/// Records every prompt the mock saw, and answers deterministically:
/// - triage turns (recognised by the triage envelope markers) → a JSON
///   decision (drop for cron, escalate otherwise);
/// - any other turn (the session/orchestrator agent) → a plain final reply.
struct MockLlm {
    seen: StdMutex<Vec<String>>,
    /// When set, the orchestrator turn emits a real `spawn_subagent` tool
    /// call so the harness runs an actual sub-agent (native tool mode).
    spawn: bool,
}

const SUBAGENT_MARKER: &str = "SUBAGENT_TASK";
const RESEARCHER_FINDINGS: &str = "Researcher findings: Q3 numbers look healthy.";

impl MockLlm {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            seen: StdMutex::new(Vec::new()),
            spawn: false,
        })
    }
    fn with_spawning() -> Arc<Self> {
        Arc::new(Self {
            seen: StdMutex::new(Vec::new()),
            spawn: true,
        })
    }

    fn triage_decision(&self, joined: &str) -> String {
        let drop = joined.contains("\"source\": \"cron\"")
            || joined.contains("\"source\":\"cron\"")
            || joined.to_lowercase().contains("ignore me");
        if drop {
            "{\"action\":\"drop\",\"reason\":\"mock: routine noise\"}".to_string()
        } else {
            "{\"action\":\"escalate\",\"target_agent\":\"orchestrator\",\
              \"prompt\":\"handle this\",\"reason\":\"mock: actionable\"}"
                .to_string()
        }
    }

    fn respond(&self, joined: &str) -> ChatResponse {
        self.seen.lock().unwrap().push(joined.to_string());
        let is_triage = joined.contains("DISPLAY_LABEL:") && joined.contains("PAYLOAD:");

        let text = if is_triage {
            self.triage_decision(joined)
        } else if joined.contains(RESEARCHER_FINDINGS) {
            // Orchestrator's follow-up turn after the sub-agent returned →
            // merge + finish. Checked BEFORE the marker because turn-2's
            // history echoes the tool-call arguments (which carry the marker).
            "Mock orchestrator merged the sub-agent findings.".to_string()
        } else if joined.contains(SUBAGENT_MARKER) {
            // The spawned researcher sub-agent's own turn → return findings,
            // no further tool calls (prevents recursive spawning).
            RESEARCHER_FINDINGS.to_string()
        } else if self.spawn {
            // Orchestrator's first turn → delegate via a real spawn_subagent
            // tool call.
            return ChatResponse {
                text: Some("Delegating to the researcher.".to_string()),
                tool_calls: vec![ToolCall {
                    id: "call-1".to_string(),
                    name: "spawn_subagent".to_string(),
                    arguments: serde_json::json!({
                        "agent_id": "researcher",
                        "prompt": format!("{SUBAGENT_MARKER}: investigate the Q3 numbers"),
                    })
                    .to_string(),
                    extra_content: None,
                }],
                usage: None,
                reasoning_content: None,
            };
        } else {
            "Mock orchestrator handled the promoted trigger.".to_string()
        };

        ChatResponse {
            text: Some(text),
            tool_calls: Vec::new(),
            usage: None,
            reasoning_content: None,
        }
    }

    fn triage_turns(&self) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.contains("DISPLAY_LABEL:") && p.contains("PAYLOAD:"))
            .count()
    }

    fn saw(&self, needle: &str) -> bool {
        self.seen.lock().unwrap().iter().any(|p| p.contains(needle))
    }
}

#[async_trait]
impl Provider for MockLlm {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            // Native tools only when we want to emit a spawn tool call.
            native_tool_calling: self.spawn,
            vision: false,
        }
    }

    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        let joined = format!("{}\n{message}", system_prompt.unwrap_or(""));
        Ok(self.respond(&joined).text.unwrap_or_default())
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        let joined = request
            .messages
            .iter()
            .map(|m| format!("{}: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(self.respond(&joined))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Hermetic harness: temp HOME + config + globals + installed mock provider.
// ─────────────────────────────────────────────────────────────────────────────

/// Serializes these tests: they mutate process-global env + install a
/// process-global provider override.
fn serial() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<StdMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| StdMutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

struct EnvGuard {
    key: &'static str,
    old: Option<String>,
}
impl EnvGuard {
    fn set(key: &'static str, val: &str) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, val);
        Self { key, old }
    }
    fn unset(key: &'static str) -> Self {
        let old = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, old }
    }
}
impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}

struct Harness {
    workspace: std::path::PathBuf,
    mock: Arc<MockLlm>,
    _install: test_provider_override::InstallGuard,
    _home: EnvGuard,
    _ws: EnvGuard,
    _keyring: EnvGuard,
    _tmp: tempfile::TempDir,
}

fn write_config(openhuman_dir: &std::path::Path) {
    // api_url is never dialed — the provider override intercepts creation
    // before any URL is used — but config must parse and disable local AI so
    // nothing reaches Ollama either.
    let cfg = r#"api_url = "http://127.0.0.1:9"
default_model = "mock-model"
default_temperature = 0.2
chat_onboarding_completed = true

[secrets]
encrypt = false

[local_ai]
enabled = false
runtime_enabled = false
"#;
    let write = |dir: &std::path::Path| {
        std::fs::create_dir_all(dir).expect("mkdir");
        std::fs::write(dir.join("config.toml"), cfg).expect("write config");
    };
    write(openhuman_dir);
    write(&openhuman_dir.join("users").join("local"));
    // Sanity: the config must match the schema.
    let _: openhuman_core::openhuman::config::Config = toml::from_str(cfg).expect("config schema");
}

fn harness() -> Harness {
    harness_with(MockLlm::new())
}

fn harness_spawning() -> Harness {
    harness_with(MockLlm::with_spawning())
}

fn harness_with(mock: Arc<MockLlm>) -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().to_path_buf();
    let openhuman_dir = home.join(".openhuman");
    write_config(&openhuman_dir);
    let workspace = home.join("workspace");
    std::fs::create_dir_all(&workspace).expect("mkdir ws");

    let home_guard = EnvGuard::set("HOME", home.to_str().unwrap());
    let ws_guard = EnvGuard::set("OPENHUMAN_WORKSPACE", workspace.to_str().unwrap());
    let keyring_guard = EnvGuard::set("OPENHUMAN_KEYRING_BACKEND", "file");

    // Globals the real pipeline needs.
    init_global(64);
    openhuman_core::openhuman::agent::bus::register_agent_handlers();
    let _ = AgentDefinitionRegistry::init_global_builtins();

    // Install the mock LLM — both provider funnels consult this first.
    let install = test_provider_override::install(mock.clone());

    Harness {
        workspace,
        mock,
        _install: install,
        _home: home_guard,
        _ws: ws_guard,
        _keyring: keyring_guard,
        _tmp: tmp,
    }
}

fn human_event(message: &str) -> DomainEvent {
    DomainEvent::ChannelInboundMessage {
        event_name: "msg".into(),
        channel: "slack".into(),
        message: message.into(),
        sender: Some("U1".into()),
        reply_target: Some("dm".into()),
        thread_ts: Some("t1".into()),
        raw_data: serde_json::Value::Null,
    }
}

fn cron_event() -> DomainEvent {
    DomainEvent::CronJobTriggered {
        job_id: "nightly".into(),
        job_name: "nightly recap".into(),
        job_type: "agent".into(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Gate (real triage pipeline) over the mock — promote vs drop.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn fullstack_gate_promotes_human_message_via_real_triage() {
    let _s = serial();
    let h = harness();
    let gate = GatePass::new(100);

    let trigger = normalize(&human_event("can you help with the Q3 plan?"), 1.0).unwrap();
    let decision = gate.evaluate(&trigger, 1.0).await;

    println!("\n[fullstack] human trigger gate decision: {decision:?}");
    assert!(
        decision.is_promote(),
        "real triage (mock-backed) should promote an actionable human message; got {decision:?}"
    );
    assert!(h.mock.triage_turns() >= 1, "the real triage LLM path ran");
}

#[tokio::test]
async fn fullstack_gate_drops_cron_noise_via_real_triage() {
    let _s = serial();
    let h = harness();
    let gate = GatePass::new(100);

    let trigger = normalize(&cron_event(), 1.0).unwrap();
    let decision = gate.evaluate(&trigger, 1.0).await;

    println!("[fullstack] cron trigger gate decision: {decision:?}");
    assert!(
        !decision.is_promote(),
        "routine cron noise should be dropped"
    );
    assert!(h.mock.triage_turns() >= 1);
}

// ─────────────────────────────────────────────────────────────────────────────
// Session (real orchestrator agent + tool loop) over the mock.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn fullstack_session_runs_real_agent_and_persists() {
    let _s = serial();
    let h = harness();

    let session = LongLivedSession::with_thread(
        h.workspace.clone(),
        SubconsciousMode::Aggressive,
        "subconscious:orchestrator".into(),
    );

    let outcome = session
        .process_promoted("[user/slack] Please look into the Q3 numbers.", false)
        .await;

    println!("[fullstack] session outcome: {outcome:?}");
    let outcome = outcome.expect("real agent turn (mock-backed) should succeed");
    assert!(
        outcome.response.contains("Mock orchestrator"),
        "session returned the mock agent's reply: {}",
        outcome.response
    );

    // Real reserved-thread persistence: the user turn + agent reply landed.
    let msgs = openhuman_core::openhuman::memory_conversations::get_messages(
        h.workspace.clone(),
        "subconscious:orchestrator",
    )
    .expect("read reserved thread");
    let senders: Vec<&str> = msgs.iter().map(|m| m.sender.as_str()).collect();
    assert!(
        senders.contains(&"user"),
        "user turn persisted: {senders:?}"
    );
    assert!(
        senders.contains(&"agent"),
        "agent reply persisted: {senders:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Full chain: human → subconscious session → REAL sub-agent → back → human.
// The mock makes the orchestrator emit a real `spawn_subagent` tool call, so
// the harness runs an actual researcher sub-agent (inheriting the mock
// provider), whose output is merged by the orchestrator's follow-up turn.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn fullstack_session_spawns_real_subagent_and_merges() {
    let _s = serial();
    let h = harness_spawning();

    let session = LongLivedSession::with_thread(
        h.workspace.clone(),
        SubconsciousMode::Aggressive,
        "subconscious:orchestrator".into(),
    );

    let outcome = session
        .process_promoted("[user/slack] Please research the Q3 numbers.", false)
        .await;

    println!("[fullstack] spawn outcome: {outcome:?}");
    let outcome = outcome.expect("orchestrator turn with a real sub-agent should succeed");

    // The orchestrator delegated and a REAL researcher sub-agent ran (its turn
    // carried our SUBAGENT_TASK marker), and its findings flowed back into the
    // session result — the full human → subconscious → sub-agent → back chain,
    // all production code, mock-backed model.
    assert!(
        h.mock.saw(SUBAGENT_MARKER),
        "the real harness ran the spawned researcher sub-agent"
    );
    assert!(
        outcome.response.contains("Researcher findings")
            || outcome.response.contains("merged the sub-agent findings"),
        "the sub-agent's output flowed back to the session: {}",
        outcome.response
    );
}
